// Viewport resolution patch for Dead Cells
//
// Patches NATIVE_WIDTH and NATIVE_HEIGHT in the static initializer
// to allow rendering at different internal resolutions with scaling.
//
// The game will render at the specified internal resolution and
// scale up to fill the framebuffer (with letterboxing if needed).
//
// Run with: cargo run -p hlbc --example patch_viewport -- input.hl output.hl [--resolution RES]

use hlbc::inject::FunctionPatcher;
use hlbc::opcodes::Opcode;
use hlbc::types::{RefFun, RefInt, Reg};
use hlbc::Bytecode;
use std::env;

#[derive(Debug, Clone, Copy)]
enum Resolution {
    Original,  // 683x400 - Original Dead Cells resolution
    Vga,       // 640x480 - VGA resolution (1:1 with common framebuffers)
    HalfVga,   // 320x240 - Half VGA (2x upscale to 640x480)
    Qvga,      // 160x120 - Quarter VGA (4x upscale to 640x480)
}

impl Resolution {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "original" | "683x400" => Some(Resolution::Original),
            "vga" | "640x480" => Some(Resolution::Vga),
            "half" | "halfvga" | "320x240" => Some(Resolution::HalfVga),
            "quarter" | "qvga" | "160x120" => Some(Resolution::Qvga),
            _ => None,
        }
    }

    fn width_const(&self) -> usize {
        match self {
            Resolution::Original => 4120, // I4120 = 683
            Resolution::Vga => 113,       // I113 = 640
            Resolution::HalfVga => 891,   // I891 = 320
            Resolution::Qvga => 61,       // I61 = 160
        }
    }

    fn height_const(&self) -> usize {
        match self {
            Resolution::Original => 700,  // I700 = 400
            Resolution::Vga => 710,       // I710 = 480
            Resolution::HalfVga => 33,    // I33 = 240
            Resolution::Qvga => 156,      // I156 = 120
        }
    }

    fn dimensions(&self) -> (u32, u32) {
        match self {
            Resolution::Original => (683, 400),
            Resolution::Vga => (640, 480),
            Resolution::HalfVga => (320, 240),
            Resolution::Qvga => (160, 120),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Resolution::Original => "Original (683x400)",
            Resolution::Vga => "VGA (640x480)",
            Resolution::HalfVga => "Half-VGA (320x240)",
            Resolution::Qvga => "QVGA (160x120)",
        }
    }
}

fn print_usage(program: &str) {
    eprintln!("Usage: {} <dead_cells.hl> <output.hl> [--resolution RES]", program);
    eprintln!();
    eprintln!("Patches NATIVE_WIDTH and NATIVE_HEIGHT in Dead Cells bytecode.");
    eprintln!();
    eprintln!("Available resolutions:");
    eprintln!("  original  683x400  Original Dead Cells resolution (no change)");
    eprintln!("  vga       640x480  VGA resolution (default, 1:1 with framebuffer)");
    eprintln!("  half      320x240  Half-VGA (2x upscale, good for testing)");
    eprintln!("  quarter   160x120  Quarter-VGA (4x upscale, very pixelated)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {} dead_cells.hl patched.hl --resolution half", program);
    eprintln!("  {} dead_cells.hl patched.hl -r 320x240", program);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    // Parse optional resolution argument
    let mut resolution = Resolution::Vga; // Default to VGA (640x480)

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "-r" | "--resolution" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --resolution requires an argument");
                    print_usage(&args[0]);
                    std::process::exit(1);
                }
                resolution = match Resolution::from_str(&args[i + 1]) {
                    Some(r) => r,
                    None => {
                        eprintln!("Error: Unknown resolution '{}'", args[i + 1]);
                        print_usage(&args[0]);
                        std::process::exit(1);
                    }
                };
                i += 2;
            }
            _ => {
                eprintln!("Error: Unknown argument '{}'", args[i]);
                print_usage(&args[0]);
                std::process::exit(1);
            }
        }
    }

    println!("Reading Dead Cells bytecode from {}...", input_path);
    let mut bytecode = Bytecode::from_file(input_path)?;

    // The static initialization function is F37330
    // It contains the initialization of Viewport.NATIVE_WIDTH and NATIVE_HEIGHT
    let static_init_findex = RefFun(37330);

    println!("\n=== Patching Viewport resolution constants ===");
    println!("Target resolution: {}", resolution.name());

    let mut patcher = FunctionPatcher::from_findex(&mut bytecode, static_init_findex)?;

    println!("Found static init function at findex {}", static_init_findex.0);
    println!("Total opcodes: {}", patcher.opcode_count());

    // Verify we're patching the right instructions
    // Instruction 23897: OInt 226, 4120 (683) -> NATIVE_WIDTH
    // Instruction 23900: OInt 226, 700 (400)  -> NATIVE_HEIGHT

    // Check instruction 23897
    match patcher.get_opcode(23897) {
        Some(Opcode::Int { dst, ptr }) if dst.0 == 226 && ptr.0 == 4120 => {
            println!("  Opcode 23897: Int r{}, I{} (683=NATIVE_WIDTH) - OK", dst.0, ptr.0);
        }
        other => {
            return Err(format!(
                "Expected Int r226, I4120 at opcode 23897, got {:?}",
                other
            ).into());
        }
    }

    // Check instruction 23900
    match patcher.get_opcode(23900) {
        Some(Opcode::Int { dst, ptr }) if dst.0 == 226 && ptr.0 == 700 => {
            println!("  Opcode 23900: Int r{}, I{} (400=NATIVE_HEIGHT) - OK", dst.0, ptr.0);
        }
        other => {
            return Err(format!(
                "Expected Int r226, I700 at opcode 23900, got {:?}",
                other
            ).into());
        }
    }

    let (new_width, new_height) = resolution.dimensions();

    // Apply the patches
    println!("\nApplying patches...");

    // Patch NATIVE_WIDTH
    patcher.replace_opcode_at(23897, Opcode::Int {
        dst: Reg(226),
        ptr: RefInt(resolution.width_const()),
    })?;
    println!("  Patched NATIVE_WIDTH: 683 -> {} (I{})", new_width, resolution.width_const());

    // Patch NATIVE_HEIGHT
    patcher.replace_opcode_at(23900, Opcode::Int {
        dst: Reg(226),
        ptr: RefInt(resolution.height_const()),
    })?;
    println!("  Patched NATIVE_HEIGHT: 400 -> {} (I{})", new_height, resolution.height_const());

    // Save patched bytecode
    println!("\nWriting patched bytecode to {}...", output_path);
    let mut file = std::fs::File::create(output_path)?;
    bytecode.serialize(&mut file)?;

    println!("\nDone! Viewport resolution patched.");
    println!("Original: 683x400 (ratio 1.7075)");
    println!("Patched:  {}x{} (ratio {:.3})",
             new_width, new_height,
             new_width as f32 / new_height as f32);

    if matches!(resolution, Resolution::HalfVga | Resolution::Qvga) {
        let scale = 640.0 / new_width as f32;
        println!();
        println!("Internal resolution {}x{} will scale {:.0}x to fill a 640x480 framebuffer.",
                 new_width, new_height, scale);
        println!("The game should letterbox/scale automatically.");
    }

    Ok(())
}
