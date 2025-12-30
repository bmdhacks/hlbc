use std::fs;
use std::io::{stdin, BufReader, BufWriter, Write};
use std::iter::repeat;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Parser as ClapParser;
use temp_dir::TempDir;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use hlbc::fmt::EnhancedFmt;
use hlbc::opcodes::Opcode;
use hlbc::types::{FunPtr, RefFun, RefGlobal, Type};
use hlbc::*;

use crate::command::{commands_parser, Command, ElementRef, FileOrIndex, ParseContext, Parser};

/// Command parser
mod command;

#[derive(ClapParser, Debug)]
#[clap(
    author,
    version,
    about = "Interactive explorer for Hashlink bytecode (.hl / hlboot.dat files)",
    long_about = "HLBC is an interactive tool for exploring, analyzing, and decompiling Hashlink \
bytecode. Hashlink is a VM for the Haxe language, used by games like Northgard, \
Dead Cells, and Wartales.\n\n\
Once loaded, you'll get an interactive prompt where you can inspect functions, \
types, strings, and other bytecode elements. Type 'help' at the prompt for a \
full list of commands.",
    after_help = "INTERACTIVE COMMANDS (type 'help' after loading for full list):
  info              Show bytecode overview
  fn <idx>          Show function with opcodes
  sfn <pattern>     Search for functions by name
  decomp <idx>      Decompile a function
  refto <type@idx>  Find references to an element
  type <idx>        Show type definition

EXAMPLES:
  hlbc game.hl                     Open bytecode interactively
  hlbc game.hl -c 'help'           Show all interactive commands
  hlbc game.hl -c 'info'           Show info and exit
  hlbc game.hl -c 'sfn update'     Search for 'update' functions
  hlbc game.hl -w 'decomp 42'      Watch file and re-decompile on change
  hlbc game.hl --gen-externs -o externs/    Generate Haxe extern definitions"
)]
struct Args {
    /// Hashlink bytecode file (.hl) or Haxe source file (.hx) to analyze
    file: PathBuf,

    /// Re-run command whenever the file changes (requires 'watch' feature)
    #[clap(short, long, value_name = "CMD")]
    watch: Option<String>,

    /// Run command immediately after loading, then exit
    #[clap(short, long, value_name = "CMD")]
    command: Option<String>,

    /// Generate Haxe extern definitions from bytecode
    #[clap(long)]
    gen_externs: bool,

    /// Output directory for generated extern files (used with --gen-externs)
    #[clap(short, long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// Filter types by pattern (e.g., "h3d.**", "game.Player") - can be specified multiple times
    #[clap(long = "type", value_name = "PATTERN")]
    type_filter: Vec<String>,

    /// Include internal/private types (starting with _ or containing $)
    #[clap(long)]
    include_internal: bool,
}

fn main() -> anyhow::Result<()> {
    let args: Args = Args::parse();

    #[cfg(not(feature = "watch"))]
    if args.watch.is_some() {
        println!("The program was not compiled with the 'watch' feature enabled.");
        return Ok(());
    }

    let tty = atty::is(atty::Stream::Stdout);

    let mut stdout = StandardStream::stdout(if tty {
        ColorChoice::Auto
    } else {
        ColorChoice::Never
    });

    let is_source = args
        .file
        .extension()
        .map(|ext| ext == "hx")
        .unwrap_or(false);

    let dir = TempDir::new()?;
    let file = if is_source {
        if tty {
            print!("Compiling haxe source ... ");
            stdout.flush()?;
        }
        let path = dir.child("bytecode.hl");
        compile(&args.file, &path)?;
        if tty {
            println!(" OK");
        }
        path
    } else {
        args.file.clone()
    };

    let start = Instant::now();

    let code = {
        let mut r = BufReader::new(fs::File::open(&file)?);
        Bytecode::deserialize(&mut r)?
    };

    if tty {
        println!("Loaded ! ({} ms)", start.elapsed().as_millis());
    }

    // Handle --gen-externs mode
    if args.gen_externs {
        use hlbc::extern_gen::{ExternGenOptions, generate_all_externs, write_externs_to_dir};

        let options = ExternGenOptions {
            type_filter: if args.type_filter.is_empty() {
                None
            } else {
                Some(args.type_filter.clone())
            },
            include_internal: args.include_internal,
            generate_native_meta: true,
        };

        let result = generate_all_externs(&code, &options);

        // Print summary
        println!("Generated {} extern files:", result.files.len());
        println!("  Classes: {}", result.class_count);
        println!("  Enums: {}", result.enum_count);
        println!("  Abstracts: {}", result.abstract_count);

        if !result.skipped.is_empty() && tty {
            println!("  Skipped: {}", result.skipped.len());
        }

        // Write to output directory
        if let Some(output_dir) = &args.output {
            write_externs_to_dir(&result, output_dir)?;
            println!("\nWritten to: {}", output_dir.display());
        } else {
            // Print to stdout if no output directory specified
            for (path, content) in &result.files {
                println!("\n=== {} ===", path);
                println!("{}", content);
            }
        }

        return Ok(());
    }

    let parse_ctx = ParseContext {
        int_max: code.ints.len(),
        float_max: code.floats.len(),
        string_max: code.strings.len(),
        debug_file_max: code.debug_files.as_ref().map(|v| v.len()).unwrap_or(0),
        type_max: code.types.len(),
        global_max: code.globals.len(),
        native_max: code.natives.len(),
        constant_max: code.constants.as_ref().map(|v| v.len()).unwrap_or(0),
        findex_max: code.findex_max(),
    };

    let parser = commands_parser(&parse_ctx);

    macro_rules! execute_commands {
        ($code:expr, $commands:expr; $onexit:stmt) => {
            for cmd in $commands {
                match cmd {
                    #[allow(redundant_semicolons)]
                    Command::Exit => {
                        $onexit;
                    }
                    cmd => {
                        process_command(&mut stdout, $code, cmd)?;
                    }
                }
                println!();
            }
        };
    }

    // Execute the -c and exit
    if let Some(initial_cmd) = args.command {
        execute_commands!(&code, parser.parse(initial_cmd.as_str()).expect("Error while parsing command."); return Ok(()));
        return Ok(());
    }

    #[cfg(feature = "watch")]
    if let Some(watch) = args.watch {
        use notify::RecursiveMode;
        use notify_debouncer_mini::new_debouncer;
        use std::sync::mpsc;

        let (tx, rx) = mpsc::channel();

        let mut debouncer = new_debouncer(Duration::from_millis(200), tx)?;

        debouncer
            .watcher()
            .watch(&args.file, RecursiveMode::NonRecursive)
            .expect("Can't watch file");

        println!("Watching file '{}', command : {watch}", args.file.display());

        let commands = parser.parse(watch.as_str()).expect("Can't parse command");

        execute_commands!(&code, commands.clone(); return Ok(()));

        'watch: loop {
            match rx.recv() {
                Ok(Ok(events)) => {
                    for e in events {
                        if is_source {
                            compile(&args.file, &file)?;
                        }

                        let code = {
                            let mut r = BufReader::new(fs::File::open(&file)?);
                            Bytecode::deserialize(&mut r)?
                        };

                        execute_commands!(&code, commands.clone(); break 'watch);
                    }
                }
                Ok(Err(e)) => {
                    println!("Error while watching : {e:?}");
                    break;
                }
                Err(e) => {
                    println!("Error while watching : {e}");
                    break;
                }
            }
        }

        return Ok(());
    }

    'main: loop {
        let mut line = String::new();
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))?;
        print!("> ");
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
        stdout.flush()?;
        stdin().read_line(&mut line)?;
        stdout.reset()?;

        let commands = parser
            .parse(line.trim())
            .expect("Error while parsing command.");
        execute_commands!(&code, commands; break 'main);
    }
    Ok(())
}

fn process_command(
    stdout: &mut StandardStream,
    code: &Bytecode,
    cmd: Command,
) -> anyhow::Result<()> {
    macro_rules! print_i {
        ($i:expr) => {
            stdout.set_color(ColorSpec::new().set_fg(Some(Color::Ansi256(242))))?;
            write!(stdout, "{:<3}: ", $i)?;
            stdout.reset()?;
        };
    }

    fn require_debug_info(code: &Bytecode) -> anyhow::Result<&[Str]> {
        if let Some(debug_files) = &code.debug_files {
            Some(&debug_files[..])
        } else {
            println!("No debug info in this binary");
            None
        }
        .context("No debug info")
    }

    match cmd {
        Command::Exit => unreachable!(),
        Command::Help => {
            println!(
                r#"HLBC - Hashlink Bytecode Explorer
==================================

GENERAL
  help                          Show this help message
  info                          Display bytecode overview (version, counts, entry point)
  entrypoint                    Show the main entry point function
  wiki                          Open the bytecode wiki in your browser
  exit                          Exit hlbc-cli

DATA INSPECTION
  i, int       <idx>            Show integer constant at index
  f, float     <idx>            Show float constant at index
  s, string    <idx>            Show string at index
  t, type      <idx>            Show type definition at index
  g, global    <idx>            Show global variable at index
  c, constant  <idx>            Show constant at index
  n, native    <idx>            Show native function binding at index
  file         <idx>            Show debug source file name at index

FUNCTIONS
  fn           <findex>         Show full function with opcodes
  fnh          <findex>         Show function header only (signature, no body)
  fnn, fnamed  <name>           Look up function by exact name
  infile       <idx|name>       List all functions defined in a source file
  fileof       <findex>         Show which source file defines a function

SEARCH
  sfn          <pattern>        Search for functions matching pattern
  sstr         <pattern>        Search for strings matching pattern
  sfile        <pattern>        Search for debug file names matching pattern

ANALYSIS
  explain      <opcode>         Show documentation for a bytecode opcode
  refto        <type@idx>       Find all references to a bytecode element
  callgraph    <findex> <depth> Generate DOT call graph from function
  decomp       <findex>         Decompile function to Haxe-like source
  decompt      <idx>            Decompile entire type (class/enum) to source
  dump-types   [prefix]         Dump all types (optionally filtered by prefix)

OUTPUT
  saveto       <filename>       Write bytecode to file (for round-trip testing)

RANGE NOTATION
  Most commands accepting <idx> support Rust-style ranges:
    string 0..10     First 10 strings (indices 0-9)
    type ..5         First 5 types
    int 100..        All ints from index 100 onward
    global ..        All globals

ELEMENT REFERENCES (for refto)
  Format: <type>@<index>
    fun@42           Function with findex 42
    type@10          Type at index 10
    global@5         Global at index 5
    string@100       String at index 100
    native@3         Native function at index 3

EXAMPLES
  > info                        Show bytecode summary
  > sfn update                  Find functions containing "update"
  > fn 42                       Show function at findex 42
  > decomp 42                   Decompile that function
  > refto fun@42                Find all calls to function 42
  > callgraph 42 3              Show call graph 3 levels deep
  > string ..20                 Show first 20 strings
  > type 0..5; global 0..3      Run multiple commands (semicolon-separated)"#
            );
        }
        Command::Explain(s) => {
            if let Some(o) = Opcode::from_name(&s) {
                println!("{} :\n{}", o.name(), o.description());
                println!("Example : {}", o.display(code, &code.functions[0], 0, 0));
            } else {
                println!("No opcode named '{s}' exists.");
            }
        }
        Command::Wiki => webbrowser::open("https://github.com/Gui-Yom/hlbc/wiki")?,
        Command::Info => {
            println!(
                "version: {}\ndebug: {}\nnints: {}\nnfloats: {}\nnstrings: {}\nntypes: {}\nnnatives: {}\nnfunctions: {}\nnconstants: {}",
                code.version,
                code.debug_files.is_some(),
                code.ints.len(),
                code.floats.len(),
                code.strings.len(),
                code.types.len(),
                code.natives.len(),
                code.functions.len(),
                code.constants.as_ref().map_or(0, |c| c.len())
            );
        }
        Command::Entrypoint => {
            println!("{}", code.entrypoint().display_header::<EnhancedFmt>(code));
        }
        Command::Int(range) => {
            for i in range {
                print_i!(i);
                println!("{}", code.ints[i]);
            }
        }
        Command::Float(range) => {
            for i in range {
                print_i!(i);
                println!("{}", code.floats[i]);
            }
        }
        Command::String(range) => {
            for i in range {
                print_i!(i);
                println!("{}", code.strings[i]);
            }
        }
        Command::SearchStr(str) => {
            for (i, s) in code.strings.iter().enumerate() {
                if s.contains(&*str) {
                    print_i!(i);
                    println!("{}", s);
                }
            }
        }
        Command::Debugfile(range) => {
            let debug_files = require_debug_info(code)?;
            for i in range {
                print_i!(i);
                println!("{}", debug_files[i]);
            }
        }
        Command::SearchDebugfile(str) => {
            let debug_files = require_debug_info(code)?;
            for (i, s) in debug_files.iter().enumerate() {
                if s.contains(&*str) {
                    print_i!(i);
                    println!("{}", s);
                }
            }
        }
        Command::Type(range) => {
            let range_len = range.len();
            for i in range {
                print_i!(i);
                let t = &code.types[i];
                println!("{}", t.display::<EnhancedFmt>(code));
                // Only display full info if selecting a single item
                if range_len == 1 {
                    match t {
                        Type::Obj(obj) => {
                            if let Some(sup) = obj.super_ {
                                println!("extends {}", sup.display::<EnhancedFmt>(code));
                            }
                            println!("global: {}", obj.global.0);
                            println!("fields:");
                            for f in &obj.own_fields {
                                println!(
                                    "  {}: {}",
                                    f.name.display::<EnhancedFmt>(code),
                                    f.t.display::<EnhancedFmt>(code)
                                );
                            }
                            println!("protos:");
                            for p in &obj.protos {
                                println!(
                                    "  {}: {} ({})",
                                    p.name.display::<EnhancedFmt>(code),
                                    code.get(p.findex).display_header::<EnhancedFmt>(code),
                                    p.pindex
                                );
                            }
                            println!("bindings:");
                            for (fi, fun) in &obj.bindings {
                                println!(
                                    "  {}: {}",
                                    fi.display::<EnhancedFmt>(code, t),
                                    fun.display_header::<EnhancedFmt>(code)
                                );
                            }
                        }
                        Type::Enum {
                            global, constructs, ..
                        } => {
                            println!("global: {}", global.0);
                            println!("constructs:");
                            for c in constructs {
                                println!("  {}:", c.name(code));
                                for (i, p) in c.params.iter().enumerate() {
                                    println!("    {i}: {}", p.display::<EnhancedFmt>(code));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Command::Global(range) => {
            for i in range {
                print_i!(i);
                println!("{}", code.globals[i].display::<EnhancedFmt>(code));
                if let Some(&cst) = code.globals_initializers.get(&RefGlobal(i)) {
                    for init in &code.constants.as_ref().unwrap()[cst].fields {
                        println!("    {}", init);
                    }
                }
            }
        }
        Command::Native(range) => {
            for i in range {
                print_i!(i);
                println!("{}", code.natives[i].display::<EnhancedFmt>(code));
            }
        }
        Command::Constant(range) => {
            for i in range {
                print_i!(i);
                println!("{:#?}", code.constants.as_ref().unwrap()[i]);
            }
        }
        Command::FunctionHeader(range) => {
            for findex in range {
                print_i!(findex);
                match code.get(RefFun(findex)) {
                    FunPtr::Fun(f) => println!("{}", f.display_header::<EnhancedFmt>(code)),
                    FunPtr::Native(n) => println!("{}", n.display::<EnhancedFmt>(code)),
                }
            }
        }
        Command::Function(range) => {
            for findex in range {
                print_i!(findex);
                match code.get(RefFun(findex)) {
                    FunPtr::Fun(f) => println!("{}", f.display::<EnhancedFmt>(code)),
                    FunPtr::Native(n) => println!("{}", n.display::<EnhancedFmt>(code)),
                }
            }
        }
        Command::FunctionNamed(str) => {
            if let Some(f) = code.function_by_name(&str) {
                println!("{}", f.display::<EnhancedFmt>(code));
            } else {
                println!("unknown '{str}'");
            }
        }
        Command::SearchFunction(str) => {
            // TODO search for function
            if let Some(f) = code.function_by_name(&str) {
                println!("{}", f.display_header::<EnhancedFmt>(code));
            } else {
                println!("unknown");
            }
        }
        Command::InFile(foi) => {
            let debug_files = require_debug_info(code)?;
            match foi {
                FileOrIndex::File(str) => {
                    if let Some(idx) = debug_files.iter().enumerate().find_map(|(i, d)| {
                        if d == str {
                            Some(i)
                        } else {
                            None
                        }
                    }) {
                        println!("Functions in file@{idx} : {}", debug_files[idx]);
                        for (i, f) in code.functions.iter().enumerate() {
                            if f.debug_info.as_ref().unwrap()[f.ops.len() - 1].0 == idx {
                                print_i!(i);
                                println!("{}", f.display_header::<EnhancedFmt>(code));
                            }
                        }
                    } else {
                        println!("File {str} not found !");
                    }
                }
                FileOrIndex::Index(idx) => {
                    println!("Functions in file@{idx} : {}", debug_files[idx]);
                    for (i, f) in code.functions.iter().enumerate() {
                        if f.debug_info.as_ref().unwrap()[f.ops.len() - 1].0 == idx {
                            print_i!(i);
                            println!("{}", f.display_header::<EnhancedFmt>(code));
                        }
                    }
                }
            }
        }
        Command::FileOf(idx) => {
            let debug_files = require_debug_info(code)?;
            match code.get(RefFun(idx)) {
                FunPtr::Fun(f) => {
                    let idx = f.debug_info.as_ref().unwrap()[f.ops.len() - 1].0;
                    println!(
                        "{} is in file@{idx} : {}",
                        f.display_header::<EnhancedFmt>(code),
                        &debug_files[idx]
                    );
                }
                FunPtr::Native(n) => {
                    println!(
                        "native {} is in the module {}",
                        n.display::<EnhancedFmt>(code),
                        n.lib(code)
                    )
                }
            }
        }
        Command::SaveTo(file) => {
            let mut w = BufWriter::new(fs::File::create(&*file)?);
            code.serialize(&mut w)?;
        }
        Command::Callgraph(idx, depth) => {
            #[cfg(feature = "graph")]
            {
                use hlbc::analysis::graph::{call_graph, display_graph};

                let graph = call_graph(code, RefFun(idx), depth);
                println!("{}", display_graph(&graph, code));
            }

            #[cfg(not(feature = "graph"))]
            {
                println!("hlbc-cli has been built without graph support. Build with feature 'graph' to enable callgraph generation");
            }
        }
        Command::RefTo(elem) => match elem {
            ElementRef::String(idx) => {
                println!(
                    "Finding references to string@{idx} : {}\n",
                    code.strings[idx]
                );
                if let Some(constants) = &code.constants {
                    for (i, c) in constants.iter().enumerate() {
                        if c.fields[0] == idx {
                            println!(
                                "constant@{i} expanding to global@{} (now also searching for global)",
                                c.global.0
                            );
                            code.ops().for_each(|(f, (i, o))| match o {
                                Opcode::GetGlobal { global, .. } => {
                                    if *global == c.global {
                                        println!(
                                            "in {} at {i}: GetGlobal",
                                            f.display_header::<EnhancedFmt>(code)
                                        );
                                    }
                                }
                                _ => {}
                            });
                            println!();
                        }
                    }
                }
                code.ops().for_each(|(f, (i, o))| match o {
                    Opcode::String { ptr, .. } => {
                        if ptr.0 == idx {
                            println!("{} at {i}: String", f.display_header::<EnhancedFmt>(code));
                        }
                    }
                    _ => {}
                });
            }
            ElementRef::Global(idx) => {
                println!(
                    "Finding references to global@{idx} : {}\n",
                    code.globals[idx].display::<EnhancedFmt>(code)
                );
                if let Some(constants) = &code.constants {
                    for (i, c) in constants.iter().enumerate() {
                        if c.global.0 == idx {
                            println!("constant@{i} : {:?}", c);
                        }
                    }
                }
                println!();

                code.ops().for_each(|(f, (i, o))| match o {
                    Opcode::GetGlobal { global, .. } | Opcode::SetGlobal { global, .. } => {
                        if global.0 == idx {
                            println!(
                                "{} at {i}: {}",
                                f.display_header::<EnhancedFmt>(code),
                                o.name()
                            );
                        }
                    }
                    _ => {}
                });
            }
            ElementRef::Fn(idx) => {
                println!(
                    "Finding references to fn@{idx} : {}\n",
                    RefFun(idx).display_header::<EnhancedFmt>(code)
                );
                code.functions
                    .iter()
                    .flat_map(|f| repeat(f).zip(f.find_fun_refs()))
                    .for_each(|(f, (i, o, fun))| {
                        if fun.0 == idx {
                            println!(
                                "{} at {i}: {}",
                                f.display_header::<EnhancedFmt>(code),
                                o.name()
                            );
                        }
                    });
            }
        },
        Command::Decomp(idx) => {
            if let Some(fun) = RefFun(idx).as_fn(code) {
                println!(
                    "{}",
                    hlbc_decompiler::decompile_function(code, fun)
                        .display(code, &hlbc_decompiler::fmt::FormatOptions::new(2))
                );
            }
        }
        Command::DecompType(idx) => {
            let ty = &code.types[idx];
            match ty {
                Type::Obj(obj) => {
                    println!("Dumping type@{idx} : {}", ty.display::<EnhancedFmt>(code));
                    println!(
                        "{}",
                        hlbc_decompiler::decompile_class(code, obj)
                            .display(code, &hlbc_decompiler::fmt::FormatOptions::new(2))
                    );
                }
                _ => println!("Type {idx} is not an obj"),
            }
        }
        Command::DumpTypes(prefix) => {
            dump_types(code, prefix.as_deref());
        }
    }
    Ok(())
}

/// Check if a type name matches any of the given prefixes
fn matches_prefix(name: &str, prefix: Option<&str>) -> bool {
    match prefix {
        Some(p) => name.starts_with(p),
        None => true, // No filter means match all
    }
}

/// Dump type information from bytecode
///
/// If prefix is provided, only types matching that prefix are shown.
/// Output format is designed for comparison and hxml generation.
fn dump_types(code: &Bytecode, prefix: Option<&str>) {
    use std::collections::BTreeMap;

    // Collect types with their field info
    let mut types_info: BTreeMap<String, (usize, Vec<String>, &'static str)> = BTreeMap::new();

    for ty in code.types.iter() {
        match ty {
            Type::Obj(obj) => {
                let name = obj.name(code).to_string();
                if matches_prefix(&name, prefix) {
                    let fields: Vec<String> = obj
                        .fields
                        .iter()
                        .map(|f| code.get(f.name).to_string())
                        .collect();
                    types_info.insert(name, (fields.len(), fields, "class"));
                }
            }
            Type::Struct(obj) => {
                let name = obj.name(code).to_string();
                if matches_prefix(&name, prefix) {
                    let fields: Vec<String> = obj
                        .fields
                        .iter()
                        .map(|f| code.get(f.name).to_string())
                        .collect();
                    types_info.insert(name, (fields.len(), fields, "struct"));
                }
            }
            Type::Enum { name, constructs, .. } => {
                let name_str = code.get(*name).to_string();
                if matches_prefix(&name_str, prefix) {
                    let constructs_info: Vec<String> = constructs
                        .iter()
                        .map(|c| c.name(code).to_string())
                        .collect();
                    types_info.insert(name_str, (constructs.len(), constructs_info, "enum"));
                }
            }
            _ => {}
        }
    }

    // Collect functions
    let mut functions: Vec<String> = Vec::new();

    for func in &code.functions {
        let method_name = code.get(func.name).to_string();
        let parent_name = func.parent.and_then(|p| {
            match &code.types[p.0] {
                Type::Obj(obj) | Type::Struct(obj) => Some(obj.name(code).to_string()),
                _ => None,
            }
        });

        let qualified_name = match parent_name {
            Some(ref p) => format!("{}.{}", p, method_name),
            None => method_name,
        };

        if matches_prefix(&qualified_name, prefix) {
            functions.push(qualified_name);
        }
    }

    // Output results
    let filter_desc = prefix.unwrap_or("all");
    println!("# Type Analysis (filter: {})", filter_desc);
    println!("# Types: {}, Functions: {}", types_info.len(), functions.len());
    println!();

    println!("[types]");
    for (name, (count, fields, kind)) in &types_info {
        if fields.len() <= 5 {
            println!("{} ({}) = {} ({})", name, kind, count, fields.join(", "));
        } else {
            println!("{} ({}) = {}", name, kind, count);
        }
    }

    println!();
    println!("[functions]");
    for name in &functions {
        println!("{}", name);
    }
}

/// Compile a Haxe source file to Hashlink bytecode by directly calling the Haxe compiler.
/// Requires having the haxe compiler in the `PATH`.
fn compile(source: &Path, bytecode: &Path) -> anyhow::Result<()> {
    let result = std::process::Command::new("haxe")
        .arg("-hl")
        .arg(bytecode)
        .arg("-main")
        .arg(source.file_name().unwrap())
        .stdin(std::process::Stdio::null())
        .current_dir(source.canonicalize().unwrap().parent().unwrap())
        .status()?;

    if result.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "Compilation failed with error : {}",
            result
        ))
    }
}
