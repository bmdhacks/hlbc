// Tool to patch Dead Cells bytecode to dump all GLSL shaders to stdout
// Run with: cargo run -p hlbc --example dump_shaders -- input.hl output.hl
//
// This patches GlDriver::compileShader to print the GLSL source code
// whenever a new shader is compiled (not loaded from cache).
//
// The patch replaces the cleanup code (shader.data.funs = null) with
// print logic, which is safe since the cleanup is just for memory optimization.
//
// Output format: Raw GLSL source printed to stdout, each shader starts with
// its #version directive which serves as a natural separator.

use hlbc::opcodes::Opcode;
use hlbc::types::{RefField, RefFun, Reg};
use hlbc::Bytecode;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.hl> <output.hl>", args[0]);
        eprintln!();
        eprintln!("Patches Dead Cells bytecode to dump GLSL shaders to stdout.");
        eprintln!("Each shader's source code is printed when compiled (not from cache).");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    println!("Reading bytecode from {}...", input_path);
    let mut code = Bytecode::from_file(input_path)?;

    // Target function: GlDriver::compileShader
    let compile_shader_findex = 30302;

    // Native function for printing: sys_print@std
    let sys_print_findex = 8366;

    let mut shader_print_patched = false;

    for fun in code.functions.iter_mut() {
        if fun.findex.0 == compile_shader_findex {
            println!(
                "Found GlDriver::compileShader at findex {}",
                compile_shader_findex
            );
            println!("  Registers: {}", fun.regs.len());
            println!("  Opcodes: {}", fun.ops.len());

            // Verify we have enough opcodes
            if fun.ops.len() < 17 {
                eprintln!("Function too short, expected at least 17 opcodes");
                std::process::exit(1);
            }

            // Verify the opcodes we're about to patch are what we expect
            // Original:
            //   13: OField 9, 2, 1      ; r9 = shader.data
            //   14: ONullCheck 9        ; nullcheck r9
            //   15: ONull 10            ; r10 = null
            //   16: OSetField 9, 0, 10  ; shader.data.funs = null

            // Check opcode 13 is Field
            match &fun.ops[13] {
                Opcode::Field { dst, obj, field } => {
                    println!(
                        "  Opcode 13: Field dst={} obj={} field={} (shader.data)",
                        dst.0, obj.0, field.0
                    );
                    if dst.0 != 9 || obj.0 != 2 || field.0 != 1 {
                        eprintln!("    Warning: Unexpected field access pattern");
                    }
                }
                other => {
                    eprintln!("  Opcode 13 is not Field: {:?}", other);
                    std::process::exit(1);
                }
            }

            // Check opcode 14 is NullCheck
            match &fun.ops[14] {
                Opcode::NullCheck { reg } => {
                    println!("  Opcode 14: NullCheck reg={}", reg.0);
                }
                other => {
                    eprintln!("  Opcode 14 is not NullCheck: {:?}", other);
                    std::process::exit(1);
                }
            }

            // Check opcode 15 is Null
            match &fun.ops[15] {
                Opcode::Null { dst } => {
                    println!("  Opcode 15: Null dst={}", dst.0);
                }
                other => {
                    eprintln!("  Opcode 15 is not Null: {:?}", other);
                    std::process::exit(1);
                }
            }

            // Check opcode 16 is SetField
            match &fun.ops[16] {
                Opcode::SetField { obj, field, src } => {
                    println!(
                        "  Opcode 16: SetField obj={} field={} src={} (shader.data.funs = null)",
                        obj.0, field.0, src.0
                    );
                }
                other => {
                    eprintln!("  Opcode 16 is not SetField: {:?}", other);
                    std::process::exit(1);
                }
            }

            // Check opcode 8 is JNotNull (jump if code already exists)
            match &fun.ops[8] {
                Opcode::JNotNull { reg, offset } => {
                    println!(
                        "  Opcode 8: JNotNull reg={} offset={} (skip code generation)",
                        reg.0, offset
                    );
                    if reg.0 != 8 || *offset != 8 {
                        eprintln!("    Warning: Unexpected JNotNull pattern");
                    }
                }
                other => {
                    eprintln!("  Opcode 8 is not JNotNull: {:?}", other);
                    std::process::exit(1);
                }
            }

            // Now apply the patch
            //
            // Original flow:
            //   8: JNotNull r8, +8 -> goto 17 (skip generation if code exists)
            //   9-12: Generate GLSL code with GlslOut::run
            //   13-16: Cleanup (shader.data.funs = null)
            //   17: Load code for GL
            //
            // Patched flow:
            //   8: JNotNull r8, +4 -> goto 13 (skip generation, but still print)
            //   9-12: Generate GLSL code (only if code was null)
            //   13-16: Print the GLSL code (ALWAYS executes now)
            //   17: Load code for GL
            //
            // This way, the shader code is printed whether it was just generated
            // or was pre-generated by the cache system.

            println!("  Patching opcode 8 to jump to print code instead of skipping it...");

            // Opcode 8: Change jump offset from +8 to +4
            // Original: if code != null, goto 17 (skip both generation AND print)
            // Patched:  if code != null, goto 13 (skip generation, but still print)
            fun.ops[8] = Opcode::JNotNull {
                reg: Reg(8),
                offset: 4, // Jump to opcode 13 instead of 17
            };
            println!("    8: JNotNull r8, +4 (now jumps to print code)");

            println!("  Patching opcodes 13-16 to print shader code...");

            // Opcode 13: NullCheck r8 (the GLSL code string)
            fun.ops[13] = Opcode::NullCheck { reg: Reg(8) };
            println!("    13: NullCheck r8 (code string)");

            // Opcode 14: Field r9 = r8.bytes (String.bytes is field index 0)
            fun.ops[14] = Opcode::Field {
                dst: Reg(9),
                obj: Reg(8),
                field: RefField(0),
            };
            println!("    14: Field r9 = r8.bytes");

            // Opcode 15: Call1 r10 = sys_print@std(r9)
            fun.ops[15] = Opcode::Call1 {
                dst: Reg(10),
                fun: RefFun(sys_print_findex),
                arg0: Reg(9),
            };
            println!("    15: Call1 sys_print@std(r9)");

            // Opcode 16: Nop (cleanup removed)
            fun.ops[16] = Opcode::Nop;
            println!("    16: Nop");

            shader_print_patched = true;
            break;
        }
    }

    if !shader_print_patched {
        eprintln!(
            "Failed to find GlDriver::compileShader at findex {}",
            compile_shader_findex
        );
        std::process::exit(1);
    }

    println!("\nWriting patched bytecode to {}...", output_path);
    let mut file = std::fs::File::create(output_path)?;
    code.serialize(&mut file)?;

    println!("Done! Patch applied:");
    println!("  - GlDriver::compileShader now prints GLSL source to stdout");
    println!();
    println!("Output format (with BYPASS_SHADER_CACHE enabled in patch_gles):");
    println!("  ERROR : Shader compiled at runtime: shaderLinker_HEXKEY - shader.Name(params...)");
    println!("  #version 310 es");
    println!("  ... vertex shader GLSL ...");
    println!("  #version 310 es");
    println!("  ... fragment shader GLSL ...");
    println!();
    println!("The ERROR message with shaderLinker_HEXKEY is printed by Dead Cells,");
    println!("followed by vertex then fragment GLSL from this patch.");
    println!("Use the hex key to populate the shader cache.");

    Ok(())
}
