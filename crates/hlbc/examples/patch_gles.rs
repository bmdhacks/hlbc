// Quick tool to patch glES and shaderVersion values in Dead Cells bytecode
// Run with: cargo run -p hlbc --example patch_gles -- input.hl output.hl
//
// Patches:
// 1. glES = 3.1 (opcode 15: Float -1 -> 3.1)
// 2. shaderVersion = 310 (opcode 168: Call math_round -> Int 310)
// 3. GlDriver constructor - remove gl.enable(GL_TEXTURE_CUBE_MAP_SEAMLESS) (opcode 174: Call1 -> Nop)
// 4. FileSystem::addPak - skip stampHash verification (opcode 12: JEq -> JAlways)
// 5. Rename "sample" -> "_sample" (GLSL ES 3.10 reserved keyword) in string table
// 6. GlDriver::resetStream - reduce streamKeep retention 2->1 frame (opcode 21: Int 1 -> Int 0)
// 7. Patch "y6:sample" -> "y6:sampl_" in bytes constants (serialized shader AST data)
//
// Optional (disabled by default - enable BYPASS_SHADER_CACHE to use):
// - CacheFile::load - skip shader cache entirely (forces runtime shader compilation)
// - CacheFile::compileRuntimeShader - force allowCompile = true

// Set to true to bypass shader cache and force runtime recompilation
const BYPASS_SHADER_CACHE: bool = true;

use hlbc::opcodes::Opcode;
use hlbc::types::{RefFloat, RefInt, Reg};
use hlbc::Bytecode;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.hl> <output.hl>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    println!("Reading bytecode from {}...", input_path);
    let mut code = Bytecode::from_file(input_path)?;

    // Add 3.1 to floats table for glES value
    let gles_value = 3.1f64;
    let new_float_idx = code.floats.len();
    code.floats.push(gles_value);
    println!("Added float {} at index {}", gles_value, new_float_idx);

    // Find the int constant index for 310 (shaderVersion)
    // In Dead Cells bytecode, int[3316] = 310
    let shader_version_int_idx = 3316;
    if code.ints.len() > shader_version_int_idx {
        println!(
            "Int constant at index {}: {}",
            shader_version_int_idx, code.ints[shader_version_int_idx]
        );
        if code.ints[shader_version_int_idx] != 310 {
            eprintln!(
                "Warning: Expected int[{}] = 310, got {}",
                shader_version_int_idx, code.ints[shader_version_int_idx]
            );
        }
    } else {
        eprintln!("Int table too small, can't find index {}", shader_version_int_idx);
        std::process::exit(1);
    }

    let mut gles_patched = false;
    let mut version_patched = false;
    let mut seamless_patched = false;
    let mut stamphash_patched = false;
    let mut sample_patched = false;
    let mut precision_patched = false;
    let mut streamkeep_patched = false;
    let mut shader_bytes_patched = 0usize;

    // Shader cache bypass patches (only used when BYPASS_SHADER_CACHE is true)
    let mut cache_load_patched = !BYPASS_SHADER_CACHE;  // Skip check if disabled
    let mut cache_compile_patched = !BYPASS_SHADER_CACHE;  // Skip check if disabled

    // Patch 7: Rename "sample" to "_sample" - reserved keyword in GLSL ES 3.10
    // String S30125 = "sample" is used as a shader variable name
    let sample_string_idx = 30125;
    if code.strings.len() > sample_string_idx {
        if code.strings[sample_string_idx].as_str() == "sample" {
            code.strings[sample_string_idx] = "_sample".into();
            println!("Patched string {}: 'sample' -> '_sample' (GLSL ES reserved keyword)", sample_string_idx);
            sample_patched = true;
        } else {
            eprintln!(
                "Warning: Expected string[{}] = 'sample', got '{}'",
                sample_string_idx, code.strings[sample_string_idx]
            );
        }
    } else {
        eprintln!("String table too small, can't find index {}", sample_string_idx);
    }

    // Patch 8: Change precision from mediump to highp for better accuracy on ARM GPUs
    // String S29954 = "precision mediump float;"
    let precision_string_idx = 29954;
    if code.strings.len() > precision_string_idx {
        if code.strings[precision_string_idx].as_str() == "precision mediump float;" {
            code.strings[precision_string_idx] = "precision highp float;".into();
            println!("Patched string {}: 'precision mediump float;' -> 'precision highp float;'", precision_string_idx);
            precision_patched = true;
        } else {
            eprintln!(
                "Warning: Expected string[{}] = 'precision mediump float;', got '{}'",
                precision_string_idx, code.strings[precision_string_idx]
            );
        }
    } else {
        eprintln!("String table too small, can't find index {}", precision_string_idx);
    }

    // Patch 9: Replace "y6:sample" with "y6:sampl_" in serialized shader strings
    // The shader AST is stored as Haxe-serialized strings (not bytes constants)
    // Variable names are embedded as "y6:sample" (Haxe serialization format)
    // Using "sampl_" (6 chars) keeps same length as "sample" for in-place replacement
    let old_pattern = "y6:sample";
    let new_pattern = "y6:sampl_";

    for (idx, s) in code.strings.iter_mut().enumerate() {
        if s.contains(old_pattern) {
            let new_str = s.replace(old_pattern, new_pattern);
            let count = s.matches(old_pattern).count();
            *s = new_str.into();
            println!("Patched string S{}: {} occurrence(s) of 'y6:sample' -> 'y6:sampl_'", idx, count);
            shader_bytes_patched += count;
        }
    }
    if shader_bytes_patched > 0 {
        println!("Total: patched {} occurrences of 'y6:sample' in shader strings", shader_bytes_patched);
    }

    // Target functions
    let gldriver_findex = 30349;  // GlDriver constructor
    let filesystem_addpak_findex = 6318;  // FileSystem::addPak
    let resetstream_findex = 30328;  // GlDriver::resetStream
    let cachefile_load_findex = 5996;  // CacheFile::load
    let cachefile_compile_findex = 6012;  // CacheFile::compileRuntimeShader

    for fun in code.functions.iter_mut() {
        // Optional: CacheFile::load - skip shader cache entirely
        // Only applied when BYPASS_SHADER_CACHE is true
        if BYPASS_SHADER_CACHE && fun.findex.0 == cachefile_load_findex {
            println!("Found CacheFile::load at findex {}", cachefile_load_findex);

            // Patch: Skip loadShaders() - pretend file never exists
            // Opcode 5: OJFalse 1, 63, 0 -> OJAlways 63, 0, 0
            if fun.ops.len() > 5 {
                match &fun.ops[5] {
                    Opcode::JFalse { cond, offset } => {
                        println!(
                            "Opcode 5: JFalse cond={} offset={} (file exists check)",
                            cond.0, offset
                        );
                        fun.ops[5] = Opcode::JAlways { offset: *offset };
                        println!("Patched opcode 5: JFalse -> JAlways (skip cache loading)");
                    }
                    other => {
                        eprintln!("Opcode 5 is not JFalse: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }

            // Patch: Skip "Missing" throw - pretend allowCompile is always true
            // Opcode 70: OJTrue 1, 4, 0 -> OJAlways 4, 0, 0
            if fun.ops.len() > 70 {
                match &fun.ops[70] {
                    Opcode::JTrue { cond, offset } => {
                        println!(
                            "Opcode 70: JTrue cond={} offset={} (allowCompile check)",
                            cond.0, offset
                        );
                        fun.ops[70] = Opcode::JAlways { offset: *offset };
                        println!("Patched opcode 70: JTrue -> JAlways (skip Missing throw)");
                        cache_load_patched = true;
                    }
                    other => {
                        eprintln!("Opcode 70 is not JTrue: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }
        }

        // Optional: CacheFile::compileRuntimeShader - force allowCompile = true
        // Only applied when BYPASS_SHADER_CACHE is true
        if BYPASS_SHADER_CACHE && fun.findex.0 == cachefile_compile_findex {
            println!("Found CacheFile::compileRuntimeShader at findex {}", cachefile_compile_findex);

            // Opcode 4: OGetThis 3, 4, 0 -> OBool 3, 1, 0 (r3 = true)
            if fun.ops.len() > 4 {
                match &fun.ops[4] {
                    Opcode::GetThis { dst, field } => {
                        println!(
                            "Opcode 4: GetThis dst={} field={} (reading allowCompile)",
                            dst.0, field.0
                        );
                        fun.ops[4] = Opcode::Bool { dst: dst.clone(), value: true };
                        println!("Patched opcode 4: GetThis -> Bool true (allowCompile = true)");
                        cache_compile_patched = true;
                    }
                    other => {
                        eprintln!("Opcode 4 is not GetThis: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }
        }

        // Patch 4: FileSystem::addPak - skip stampHash verification
        // Opcode 12: OJEq 8, 9, 1 -> OJAlways 1, 0, 0 (always skip to continue loading)
        // This bypasses the res.pak stampHash check that ties pak files to specific game builds
        if fun.findex.0 == filesystem_addpak_findex {
            println!("Found FileSystem::addPak at findex {}", filesystem_addpak_findex);

            if fun.ops.len() > 12 {
                match &fun.ops[12] {
                    Opcode::JEq { a: _, b: _, offset } => {
                        println!(
                            "Opcode 12: JEq (stampHash comparison) offset={}",
                            offset
                        );

                        // Change conditional jump to unconditional - always continue loading pak
                        fun.ops[12] = Opcode::JAlways { offset: *offset };
                        println!("Patched opcode 12: JEq -> JAlways (bypass stampHash check)");
                        stamphash_patched = true;
                    }
                    other => {
                        eprintln!("Opcode 12 is not JEq: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }
        }

        // Patch 6: GlDriver::resetStream - reduce streamKeep retention from 2 frames to 1 frame
        // Original: streamKeep[0].f < frame - 1 (keep buffers for 2 frames)
        // Patched:  streamKeep[0].f < frame     (keep buffers for 1 frame)
        // Opcode 21: OInt 7, 13, 0 (r7 = 1) -> OInt 7, 1, 0 (r7 = 0)
        // This reduces memory usage by releasing old stream buffers sooner
        if fun.findex.0 == resetstream_findex {
            println!("Found GlDriver::resetStream at findex {}", resetstream_findex);

            if fun.ops.len() > 21 {
                match &fun.ops[21] {
                    Opcode::Int { dst, ptr } => {
                        // Verify it's loading 1 (ints[13] = 1)
                        if ptr.0 == 13 && code.ints[ptr.0] == 1 {
                            println!(
                                "Opcode 21: Int dst={} ptr={} (ints[{}]={})",
                                dst.0, ptr.0, ptr.0, code.ints[ptr.0]
                            );

                            // Change to load 0 instead of 1 (ints[1] = 0)
                            // This makes the condition: streamKeep[0].f < frame - 0 = frame
                            fun.ops[21] = Opcode::Int {
                                dst: dst.clone(),
                                ptr: RefInt(1),  // ints[1] = 0
                            };
                            println!("Patched opcode 21: frame - 1 -> frame - 0 (1-frame retention)");
                            streamkeep_patched = true;
                        } else {
                            eprintln!(
                                "Opcode 21 Int ptr is not 13 or value is not 1: ptr={}, value={}",
                                ptr.0, code.ints[ptr.0]
                            );
                            std::process::exit(1);
                        }
                    }
                    other => {
                        eprintln!("Opcode 21 is not Int: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }
        }

        if fun.findex.0 == gldriver_findex {
            println!("Found GlDriver constructor at findex {}", gldriver_findex);

            // Patch 1: Opcode 15 - Float that loads -1 into r8 for glES
            if fun.ops.len() > 15 {
                match &fun.ops[15] {
                    Opcode::Float { dst, ptr } => {
                        println!(
                            "Opcode 15: Float dst={} ptr={} (float[{}]={})",
                            dst.0, ptr.0, ptr.0, code.floats[ptr.0]
                        );

                        fun.ops[15] = Opcode::Float {
                            dst: dst.clone(),
                            ptr: RefFloat(new_float_idx),
                        };
                        println!("Patched opcode 15: glES = {}", gles_value);
                        gles_patched = true;
                    }
                    other => {
                        eprintln!("Opcode 15 is not Float: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }

            // Patch 2: Opcode 168 - Call math_round -> Int 310 for shaderVersion
            // Original: OCall1 6, 6266, 8 (r6 = math_round(r8))
            // Patched:  OInt 6, 3316, 0 (r6 = 310)
            if fun.ops.len() > 168 {
                match &fun.ops[168] {
                    Opcode::Call1 { dst, fun: call_fun, arg0 } => {
                        println!(
                            "Opcode 168: Call1 dst={} fun={} arg0={}",
                            dst.0, call_fun.0, arg0.0
                        );

                        fun.ops[168] = Opcode::Int {
                            dst: Reg(dst.0),
                            ptr: RefInt(shader_version_int_idx),
                        };
                        println!("Patched opcode 168: shaderVersion = 310");
                        version_patched = true;
                    }
                    other => {
                        eprintln!("Opcode 168 is not Call1: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }

            // Patch 5: Remove gl.enable(GL_TEXTURE_CUBE_MAP_SEAMLESS) - not supported on GLES
            // Opcode 174: OCall1 11, 30356, 6 -> ONop (gl_enable@sdl call removed)
            if fun.ops.len() > 174 {
                match &fun.ops[174] {
                    Opcode::Call1 { dst, fun: call_fun, arg0 } => {
                        // Verify it's calling gl_enable (findex 30356)
                        if call_fun.0 == 30356 {
                            println!(
                                "Opcode 174: Call1 dst={} fun={} arg0={} (gl_enable SEAMLESS)",
                                dst.0, call_fun.0, arg0.0
                            );

                            fun.ops[174] = Opcode::Nop;
                            println!("Patched opcode 174: gl.enable(SEAMLESS) -> Nop");
                            seamless_patched = true;
                        } else {
                            eprintln!("Opcode 174 Call1 fun is not 30356 (gl_enable): {}", call_fun.0);
                            std::process::exit(1);
                        }
                    }
                    other => {
                        eprintln!("Opcode 174 is not Call1: {:?}", other);
                        std::process::exit(1);
                    }
                }
            }

        }

        // Exit early once all function patches are applied
        if gles_patched && version_patched && seamless_patched && stamphash_patched && streamkeep_patched && cache_load_patched && cache_compile_patched {
            break;
        }
    }

    if !gles_patched {
        eprintln!("Failed to patch glES");
        std::process::exit(1);
    }
    if !version_patched {
        eprintln!("Failed to patch shaderVersion");
        std::process::exit(1);
    }
    if !seamless_patched {
        eprintln!("Failed to patch GL_TEXTURE_CUBE_MAP_SEAMLESS");
        std::process::exit(1);
    }
    if !cache_load_patched {
        eprintln!("Failed to patch CacheFile::load");
        std::process::exit(1);
    }
    if !cache_compile_patched {
        eprintln!("Failed to patch CacheFile::compileRuntimeShader");
        std::process::exit(1);
    }
    if !stamphash_patched {
        eprintln!("Failed to patch FileSystem::addPak (stampHash bypass)");
        std::process::exit(1);
    }
    if !sample_patched {
        eprintln!("Failed to patch 'sample' reserved keyword");
        std::process::exit(1);
    }
    if !precision_patched {
        eprintln!("Failed to patch 'precision mediump float;' -> 'precision highp float;'");
        std::process::exit(1);
    }
    if !streamkeep_patched {
        eprintln!("Failed to patch GlDriver::resetStream (streamKeep retention)");
        std::process::exit(1);
    }
    if shader_bytes_patched == 0 {
        eprintln!("Warning: No 'y6:sample' found in bytes constants (expected 2 in shader AST data)");
    } else if shader_bytes_patched < 2 {
        eprintln!("Warning: Only {} 'y6:sample' found in bytes (expected 2)", shader_bytes_patched);
    }

    println!("\nWriting patched bytecode to {}...", output_path);
    let mut file = std::fs::File::create(output_path)?;
    code.serialize(&mut file)?;

    println!("Done! Patches applied:");
    println!("  - glES = 3.1 (forces GLES mode)");
    println!("  - shaderVersion = 310 (outputs '#version 310 es')");
    println!("  - gl.enable(GL_TEXTURE_CUBE_MAP_SEAMLESS) removed (not supported on GLES)");
    println!("  - FileSystem::addPak: stampHash bypass (allows modified res.pak files)");
    println!("  - 'sample' -> '_sample' in string table (GLSL ES 3.10 reserved keyword)");
    println!("  - 'y6:sample' -> 'y6:sampl_' in {} bytes constants (serialized shader AST)", shader_bytes_patched);
    println!("  - 'precision mediump float;' -> 'precision highp float;' (better accuracy)");
    println!("  - GlDriver::resetStream: streamKeep retention 2->1 frame (reduces memory)");
    if BYPASS_SHADER_CACHE {
        println!("  - CacheFile::load: shader cache bypassed (forces runtime compilation)");
        println!("  - CacheFile::compileRuntimeShader: allowCompile forced true");
    }
    Ok(())
}
