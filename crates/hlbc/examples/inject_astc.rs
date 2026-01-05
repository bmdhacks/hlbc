// ASTC texture loading injection tool for Dead Cells
//
// This tool:
// 1. Injects the AstcLoader.tryLoadAstc function from astc_loader.hl
// 2. Patches Image.toTexture (F5967) to try ASTC loading first
//
// Run with: cargo run -p hlbc --example inject_astc -- input.hl astc_loader.hl output.hl

use hlbc::inject::{FunctionPatcher, InjectionError};
use hlbc::inject::merge::PoolMerger;
use hlbc::opcodes::Opcode;
use hlbc::types::{RefFun, Reg};
use hlbc::Bytecode;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!(
            "Usage: {} <dead_cells.hl> <astc_loader.hl> <output.hl>",
            args[0]
        );
        std::process::exit(1);
    }

    let input_path = &args[1];
    let astc_path = &args[2];
    let output_path = &args[3];

    println!("Reading Dead Cells bytecode from {}...", input_path);
    let mut target = Bytecode::from_file(input_path)?;

    println!("Reading ASTC loader bytecode from {}...", astc_path);
    let source = Bytecode::from_file(astc_path)?;

    // Phase 1: Find and inject the tryLoadAstc function
    // Static functions in Haxe are stored as fields on the $ClassName type,
    // not as protos, so we need to find it by source file and signature
    println!("\n=== Phase 1: Injecting ASTC loader function ===");

    // Find the tryLoadAstc function in source by looking for functions from AstcLoader.hx
    // that take a String and return a Texture
    let source_func_idx = source.functions.iter()
        .position(|f| {
            if let Some(ref debug) = f.debug_info {
                if !debug.is_empty() {
                    let (file_idx, _) = debug[0];
                    if let Some(files) = &source.debug_files {
                        if let Some(file) = files.get(file_idx as usize) {
                            return file.contains("AstcLoader.hx");
                        }
                    }
                }
            }
            false
        })
        .ok_or("Could not find tryLoadAstc function in source bytecode")?;

    let source_fref = source.functions[source_func_idx].findex;
    println!("Found tryLoadAstc in source at findex {}", source_fref.0);

    // Use PoolMerger to inject the function and all its dependencies
    let mut merger = PoolMerger::with_native_injection(&mut target, &source, true);
    let try_load_fn = merger.ensure_fun(source_fref);

    println!("Injected tryLoadAstc at target findex {}", try_load_fn.0);
    println!("Injection complete:");
    println!("  - Functions injected: {}", merger.injected_functions.len());
    println!("  - Natives injected: {}", merger.injected_natives.len());
    if !merger.warnings.is_empty() {
        println!("  - Warnings:");
        for w in &merger.warnings {
            println!("    {}", w);
        }
    }
    if !merger.type_mismatches.is_empty() {
        println!("  - Type mismatches (may cause runtime issues):");
        for tm in merger.type_mismatches.values() {
            println!("    {:?}", tm);
        }
    }

    // Phase 2: Patch Image.toTexture (F5967) to try ASTC first
    println!("\n=== Phase 2a: Patching Image.toTexture (F5967) ===");
    patch_to_texture(&mut target, try_load_fn)?;

    // Phase 2b: Patch ImageExtender.toTexture (F6223) - Dead Cells custom path for Atlas
    println!("\n=== Phase 2b: Patching ImageExtender.toTexture (F6223) ===");
    patch_image_extender(&mut target, try_load_fn)?;

    // Save patched bytecode
    println!("\nWriting patched bytecode to {}...", output_path);
    let mut file = std::fs::File::create(output_path)?;
    target.serialize(&mut file)?;

    println!("\nDone! ASTC texture loading has been injected.");
    println!("Patches applied:");
    println!("  - Injected astc.AstcLoader.tryLoadAstc function");
    println!("  - Patched hxd.res.Image.toTexture (F5967) for single images");
    println!("  - Patched ImageExtender.toTexture (F6223) for Atlas textures");

    Ok(())
}

/// Patch Image.toTexture to try ASTC loading before PNG loading.
///
/// Original F5967 start:
///   0: OGetThis 2, 1        ; r2 = this.tex
///   1: OJNull 2, 2          ; if r2 == null goto 4
///   2: OGetThis 2, 1        ; r2 = this.tex
///   3: ORet 2               ; return r2
///   4: ...                  ; actual loading code
///
/// We insert ASTC check at position 4 (after the cached texture check).
/// This ensures we still return cached textures, but try ASTC before PNG loading.
///
/// Inserted code:
///   4: OGetThis r2, 0       ; r2 = this.entry
///   5: ONullCheck r2        ; null check entry
///   6: OCallMethod r3, 14, 1; r3 = entry.get_path() (proto 14 on FileEntry)
///   7: OCall1 r4, <fn>, r3  ; r4 = tryLoadAstc(path)
///   8: OJNull r4, 4         ; if r4 == null goto original code (jump over 9-11)
///   9: OSetThis 1, r4       ; this.tex = r4
///  10: ORet r4              ; return r4
///  11: ... original code continues
fn patch_to_texture(bytecode: &mut Bytecode, try_load_fn: RefFun) -> Result<(), InjectionError> {
    // Image.toTexture is F5967
    let to_texture_findex = RefFun(5967);

    let mut patcher = FunctionPatcher::from_findex(bytecode, to_texture_findex)?;

    println!("Found Image.toTexture at findex 5967");
    println!("Original opcode count: {}", patcher.opcode_count());

    // Verify the function structure matches what we expect
    match patcher.get_opcode(0) {
        Some(Opcode::GetThis { dst, field }) if dst.0 == 2 && field.0 == 1 => {
            println!("  Opcode 0: GetThis r{}, field {} (this.tex) - OK", dst.0, field.0);
        }
        other => {
            return Err(InjectionError::TypeMismatch {
                description: format!(
                    "Expected GetThis r2, 1 at opcode 0, got {:?}",
                    other
                ),
            });
        }
    }

    // Insert ASTC loading code at position 4 (after cache check, before loading)
    // We'll insert 7 opcodes total
    let insert_pos = 4;

    // IMPORTANT: We must use registers that have compatible types with our operations.
    // The original function has 24 registers. Looking at the original code:
    // - r0: this (Image)
    // - r1: used for various results
    // - r2: Texture type (for cache and return)
    // - r3-r6: mixed int/object usage
    // - r22, r23: used for objects near end of function
    //
    // Register types from original F5967 (verified from opcodes 54-57):
    // - r23: FileEntry type (original: GetThis 23, 0 stores entry)
    // - r22: String type (original: CallMethod result passed to setName)
    // - r2: Texture type (for cache and return)
    // F5963 is Image::getSize - we need to call it to populate this.inf
    let get_size_fn = RefFun(5963);

    let opcodes = vec![
        // r23 = this.entry (field 0: inherited fields come first, so entry=0, tex=1, inf=2)
        Opcode::GetThis {
            dst: Reg(23),
            field: hlbc::types::RefField(0),
        },
        // Null check entry
        Opcode::NullCheck { reg: Reg(23) },
        // r22 = entry.get_path() - virtual call using correct pindex
        // IMPORTANT: CallMethod field is the PINDEX (vtable slot), not the proto array index!
        // hldump shows "P14: get_path" but that's the array index - actual pindex=11
        // FileEntry P14 (pindex=11): get_path -> F1611
        // PakEntry  P0  (pindex=11): get_path -> F6324
        // Both use vtable slot 11, so virtual dispatch works correctly
        Opcode::CallMethod {
            dst: Reg(22),
            field: hlbc::types::RefField(11), // pindex=11 for get_path
            args: vec![Reg(23)],              // entry object
        },
        // r2 = tryLoadAstc(r22) - r2 is Texture type, matches our return
        Opcode::Call1 {
            dst: Reg(2),
            fun: try_load_fn,
            arg0: Reg(22),
        },
        // if r2 == null, jump over getSize+SetThis+Ret to continue to original code
        // JNull offset=3 means: position 8 + 3 + 1 = position 12 (original getSize call)
        Opcode::JNull {
            reg: Reg(2),
            offset: 3, // Skip getSize (pos 9), SetThis (pos 10), Ret (pos 11), land on original code (pos 12)
        },
        // Call getSize(this) to populate this.inf - IMPORTANT for atlas processing!
        // Result goes to r3 (matches original code), but we don't use it
        Opcode::Call1 {
            dst: Reg(3),
            fun: get_size_fn,
            arg0: Reg(0), // r0 = this (Image)
        },
        // this.tex = r2 (field 1 in Image is tex)
        Opcode::SetThis {
            field: hlbc::types::RefField(1),
            src: Reg(2),
        },
        // return r2
        Opcode::Ret { ret: Reg(2) },
    ];

    let num_inserted = opcodes.len();
    println!("Inserting {} opcodes at position {}", num_inserted, insert_pos);

    patcher.insert_opcodes_at(insert_pos, opcodes)?;

    println!("New opcode count: {}", patcher.opcode_count());
    println!("Image.toTexture patched successfully");

    Ok(())
}

/// Patch ImageExtender.toTexture (F6223) - Dead Cells custom texture loading for Atlas.
///
/// Original F6223 start:
///   0: ONullCheck       0              ; null check this
///   1: OField           2, 0, 1        ; r2 = this.tex
///   2: OJNull           2, 2           ; if r2 == null goto 5
///   3: OField           2, 0, 1        ; r2 = this.tex
///   4: ORet             2              ; return r2
///   5: OCall1           3, 5963, 0     ; r3 = getSize(this)
///   ... texture creation and loadTexture ...
///
/// We insert ASTC check at position 5 (after cache check, before getSize).
fn patch_image_extender(bytecode: &mut Bytecode, try_load_fn: RefFun) -> Result<(), InjectionError> {
    let image_extender_findex = RefFun(6223);

    let mut patcher = FunctionPatcher::from_findex(bytecode, image_extender_findex)?;

    println!("Found ImageExtender.toTexture at findex 6223");
    println!("Original opcode count: {}", patcher.opcode_count());

    // Verify structure - should have cache check followed by getSize call
    match patcher.get_opcode(5) {
        Some(Opcode::Call1 { dst, fun, arg0 }) if dst.0 == 3 && fun.0 == 5963 && arg0.0 == 0 => {
            println!("  Opcode 5: Call1 r{}, F{}, r{} (getSize) - OK", dst.0, fun.0, arg0.0);
        }
        other => {
            return Err(InjectionError::TypeMismatch {
                description: format!(
                    "Expected Call1 r3, F5963, r0 (getSize) at opcode 5, got {:?}",
                    other
                ),
            });
        }
    }

    let insert_pos = 5;
    let get_size_fn = RefFun(5963);

    // Use registers r22, r21 - same types as original opcodes 55-57:
    // r22 = this.entry (FileEntry type, used at original opcode 55)
    // r21 = entry.get_path() result (String type, used at original opcode 57)
    let opcodes = vec![
        // r22 = this.entry (field 0 from Resource base class)
        Opcode::GetThis {
            dst: Reg(22),
            field: hlbc::types::RefField(0),
        },
        // Null check entry
        Opcode::NullCheck { reg: Reg(22) },
        // r21 = entry.get_path() - pindex 11
        Opcode::CallMethod {
            dst: Reg(21),
            field: hlbc::types::RefField(11),
            args: vec![Reg(22)],
        },
        // r2 = tryLoadAstc(r21)
        Opcode::Call1 {
            dst: Reg(2),
            fun: try_load_fn,
            arg0: Reg(21),
        },
        // if r2 == null, jump to original code (skip getSize, SetField, Ret)
        Opcode::JNull {
            reg: Reg(2),
            offset: 3,
        },
        // Call getSize(this) to populate this.inf
        Opcode::Call1 {
            dst: Reg(3),
            fun: get_size_fn,
            arg0: Reg(0),
        },
        // this.tex = r2 (field 1)
        Opcode::SetField {
            obj: Reg(0),
            field: hlbc::types::RefField(1),
            src: Reg(2),
        },
        // return r2
        Opcode::Ret { ret: Reg(2) },
    ];

    let num_inserted = opcodes.len();
    println!("Inserting {} opcodes at position {}", num_inserted, insert_pos);

    patcher.insert_opcodes_at(insert_pos, opcodes)?;

    println!("New opcode count: {}", patcher.opcode_count());
    println!("ImageExtender.toTexture patched successfully");

    Ok(())
}
