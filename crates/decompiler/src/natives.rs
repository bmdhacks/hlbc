//! Native function name lookup table.
//!
//! Maps HashLink native function names (lib/name) to their Haxe equivalents (Class.method).
//! Generated from Haxe stdlib @:hlNative annotations.
//!
//! Also provides dynamic binding lookup from bytecode type bindings.

use std::cell::RefCell;
use std::collections::HashMap;

use hlbc::types::RefFun;
use hlbc::{Bytecode, Resolve};

// Thread-local cache for native binding maps.
// Stores (bytecode_ptr, binding_map) to detect when we need to rebuild.
thread_local! {
    static NATIVE_BINDING_CACHE: RefCell<Option<(usize, HashMap<usize, (String, String)>)>> = RefCell::new(None);
}

/// Known library prefixes for native functions.
/// Maps (library_name, native_prefix) -> haxe_class_name
const NATIVE_PREFIX_MAP: &[(&str, &str, &str)] = &[
    ("sdl", "gl_", "sdl.GL"),
    ("sdl", "win_", "sdl.Window"),
    ("sdl", "gctrl_", "sdl.GameController"),
    ("sdl", "", "sdl.Sdl"),  // Empty prefix for base SDL functions
    ("openal", "al_", "openal.AL"),
    ("openal", "alc_", "openal.ALC"),
    ("mesa", "gl_", "mesa.GL"),
    ("directx", "", "dx.Driver"),
];

/// Convert snake_case to camelCase.
/// e.g., "create_framebuffer" -> "createFramebuffer"
fn snake_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;

    for c in s.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// Build a reverse mapping from native function references to their Haxe class.method names.
/// Uses a combination of:
/// 1. Type bindings (for wrapper functions that have implementations)
/// 2. Name convention matching (for pure natives using known prefixes)
fn build_native_binding_map(code: &Bytecode) -> HashMap<usize, (String, String)> {
    let mut bindings = HashMap::new();

    // First, collect bindings from types (covers wrapper functions)
    for ty in code.types.iter() {
        if let Some(obj) = ty.get_type_obj() {
            let class_name = code.get(obj.name).to_string();

            // Only process static holder types ($ClassName)
            if !class_name.contains('$') {
                continue;
            }

            // Clean class name: "sdl.$GL" → "sdl.GL"
            let clean_name = if class_name.starts_with('$') {
                class_name[1..].to_string()
            } else {
                class_name.replace(".$", ".")
            };

            for (&field_idx, &fun_ref) in &obj.bindings {
                // Capture ALL bindings (both natives and wrapper functions)
                if let Some(field) = obj.fields.get(field_idx.0) {
                    let method_name = code.get(field.name).to_string();
                    bindings.insert(fun_ref.0, (clean_name.clone(), method_name));
                }
            }
        }
    }

    // Second, map native functions using prefix conventions
    for native in &code.natives {
        // Skip if already mapped via bindings
        if bindings.contains_key(&native.findex.0) {
            continue;
        }

        let lib = native.lib(code);
        let name = native.name(code);

        // Strip optional '?' prefix from library name
        let lib = lib.strip_prefix('?').unwrap_or(&lib);

        // Try to match against known prefix mappings
        for &(map_lib, prefix, class_name) in NATIVE_PREFIX_MAP {
            if lib == map_lib && name.starts_with(prefix) {
                let method_snake = &name[prefix.len()..];
                let method_name = snake_to_camel(method_snake);
                bindings.insert(native.findex.0, (class_name.to_string(), method_name));
                break;
            }
        }
    }

    bindings
}

/// Look up a native function's Haxe class.method name from bytecode bindings.
/// This discovers mappings dynamically from type bindings, covering all libraries.
pub fn lookup_native_binding(code: &Bytecode, fun: RefFun) -> Option<(String, String)> {
    let bytecode_ptr = code as *const Bytecode as usize;

    NATIVE_BINDING_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // Check if we need to rebuild the cache (different bytecode)
        let needs_rebuild = match &*cache {
            Some((cached_ptr, _)) => *cached_ptr != bytecode_ptr,
            None => true,
        };

        if needs_rebuild {
            let map = build_native_binding_map(code);
            *cache = Some((bytecode_ptr, map));
        }

        // Look up the function in the cached map
        cache
            .as_ref()
            .and_then(|(_, map)| map.get(&fun.0).cloned())
    })
}

/// Look up the Haxe equivalent for a native function.
/// Returns Some("Class.method") if found, None otherwise.
pub fn lookup_native(lib: &str, name: &str) -> Option<&'static str> {
    match (lib, name) {
        // Math functions (std library)
        ("std", "math_abs") => Some("Math.abs"),
        ("std", "math_acos") => Some("Math.acos"),
        ("std", "math_asin") => Some("Math.asin"),
        ("std", "math_atan") => Some("Math.atan"),
        ("std", "math_atan2") => Some("Math.atan2"),
        ("std", "math_ceil") => Some("Math.ceil"),
        ("std", "math_cos") => Some("Math.cos"),
        ("std", "math_exp") => Some("Math.exp"),
        ("std", "math_fceil") => Some("Math.fceil"),
        ("std", "math_ffloor") => Some("Math.ffloor"),
        ("std", "math_floor") => Some("Math.floor"),
        ("std", "math_fround") => Some("Math.fround"),
        ("std", "math_isfinite") => Some("Math.isFinite"),
        ("std", "math_isnan") => Some("Math.isNaN"),
        ("std", "math_log") => Some("Math.log"),
        ("std", "math_pow") => Some("Math.pow"),
        ("std", "math_round") => Some("Math.round"),
        ("std", "math_sin") => Some("Math.sin"),
        ("std", "math_sqrt") => Some("Math.sqrt"),
        ("std", "math_tan") => Some("Math.tan"),

        // Sys functions
        ("std", "sys_args") => Some("Sys.args"),
        ("std", "sys_command") => Some("Sys.command"),
        ("std", "sys_cpu_time") => Some("Sys.cpuTime"),
        ("std", "sys_exe_path") => Some("Sys.programPath"),
        ("std", "sys_exit") => Some("Sys.exit"),
        ("std", "sys_get_char") => Some("Sys.getChar"),
        ("std", "sys_get_cwd") => Some("Sys.getCwd"),
        ("std", "sys_get_env") => Some("Sys.getEnv"),
        ("std", "sys_print") => Some("Sys.print"),
        ("std", "sys_put_env") => Some("Sys.putEnv"),
        ("std", "sys_set_cwd") => Some("Sys.setCwd"),
        ("std", "sys_sleep") => Some("Sys.sleep"),
        ("std", "sys_string") => Some("Sys.systemName"),
        ("std", "sys_time") => Some("Sys.time"),
        ("std", "sys_utf8_path") => Some("Sys.utf8Path"),
        ("std", "sys_locale") => Some("Sys.locale"),
        ("std", "sys_hl_file") => Some("Sys.hlFile"),
        ("std", "sys_is64") => Some("Sys.is64"),

        // FileSystem functions
        ("std", "sys_create_dir") => Some("FileSystem.createDirectory"),
        ("std", "sys_delete") => Some("FileSystem.deleteFile"),
        ("std", "sys_exists") => Some("FileSystem.exists"),
        ("std", "sys_full_path") => Some("FileSystem.fullPath"),
        ("std", "sys_is_dir") => Some("FileSystem.isDirectory"),
        ("std", "sys_read_dir") => Some("FileSystem.readDirectory"),
        ("std", "sys_rename") => Some("FileSystem.rename"),
        ("std", "sys_stat") => Some("FileSystem.stat"),

        // File functions
        ("std", "file_contents") => Some("File.getContent"),
        ("std", "file_open") => Some("File.open"),
        ("std", "file_close") => Some("FileInput.close"),
        ("std", "file_eof") => Some("FileInput.eof"),
        ("std", "file_read") => Some("FileInput.read"),
        ("std", "file_read_char") => Some("FileInput.readByte"),
        ("std", "file_seek") => Some("FileInput.seek"),
        ("std", "file_tell") => Some("FileInput.tell"),
        ("std", "file_flush") => Some("FileOutput.flush"),
        ("std", "file_write") => Some("FileOutput.write"),
        ("std", "file_write_char") => Some("FileOutput.writeByte"),
        ("std", "file_stdin") => Some("Sys.stdin"),
        ("std", "file_stdout") => Some("Sys.stdout"),
        ("std", "file_stderr") => Some("Sys.stderr"),

        // Bytes functions - use fully qualified haxe.io.Bytes
        ("std", "alloc_bytes") => Some("haxe.io.Bytes.alloc"),
        ("std", "bytes_blit") => Some("haxe.io.Bytes.blit"),
        ("std", "bytes_compare") => Some("haxe.io.Bytes.compare"),
        ("std", "bytes_compare16") => Some("haxe.io.Bytes.compare16"),
        ("std", "bytes_fill") => Some("haxe.io.Bytes.fill"),
        ("std", "bytes_find") => Some("haxe.io.Bytes.find"),
        ("std", "bytes_offset") => Some("haxe.io.Bytes.offset"),
        ("std", "bytes_subtract") => Some("haxe.io.Bytes.subtract"),
        ("std", "bytes_address") => Some("haxe.io.Bytes.getAddress"),
        ("std", "bytes_from_address") => Some("haxe.io.Bytes.fromAddress"),
        ("std", "bsort_i32") => Some("haxe.io.Bytes.sortI32"),
        ("std", "bsort_f64") => Some("haxe.io.Bytes.sortF64"),
        ("std", "hash") => Some("haxe.io.Bytes.hash"),
        ("std", "parse_float") => Some("haxe.io.Bytes.parseFloat"),
        ("std", "parse_int") => Some("haxe.io.Bytes.parseInt"),
        ("std", "ucs2length") => Some("haxe.io.Bytes.ucs2Length"),
        ("std", "ucs2_lower") => Some("haxe.io.Bytes.lower"),
        ("std", "ucs2_upper") => Some("haxe.io.Bytes.upper"),
        ("std", "url_decode") => Some("haxe.io.Bytes.urlDecode"),
        ("std", "url_encode") => Some("haxe.io.Bytes.urlEncode"),
        ("std", "utf16_to_utf8") => Some("haxe.io.Bytes.utf16ToUtf8"),
        ("std", "utf8_to_utf16") => Some("haxe.io.Bytes.utf8ToUtf16"),

        // Type functions
        ("std", "alloc_enum_dyn") => Some("Type.createEnumIndex"),
        ("std", "alloc_obj") => Some("Type.createEmptyInstance"),
        ("std", "type_args_count") => Some("Type.getArgsCount"),
        ("std", "type_enum_eq") => Some("Type.enumEq"),
        ("std", "type_enum_fields") => Some("Type.getEnumConstructs"),
        ("std", "type_enum_values") => Some("Type.enumParameters"),
        ("std", "type_get_global") => Some("Type.getGlobal"),
        ("std", "type_instance_fields") => Some("Type.getInstanceFields"),
        ("std", "type_name") => Some("Type.getClassName"),
        ("std", "type_safe_cast") => Some("Type.safeCast"),
        ("std", "type_set_global") => Some("Type.setGlobal"),
        ("std", "type_super") => Some("Type.getSuperClass"),

        // Reflect functions
        ("std", "obj_fields") => Some("Reflect.fields"),
        ("std", "obj_get_field") => Some("Reflect.field"),
        ("std", "obj_has_field") => Some("Reflect.hasField"),
        ("std", "obj_set_field") => Some("Reflect.setField"),
        ("std", "obj_delete_field") => Some("Reflect.deleteField"),
        ("std", "call_method") => Some("Reflect.callMethod"),

        // EReg functions
        ("std", "regexp_match") => Some("EReg.match"),
        ("std", "regexp_matched_pos") => Some("EReg.matchedPos"),
        ("std", "regexp_new_options") => Some("EReg.new"),

        // Std functions
        ("std", "rnd_float") => Some("Std.random"),
        ("std", "rnd_int") => Some("Std.randomInt"),
        ("std", "rnd_init_system") => Some("Std.initRandom"),

        // Mutex functions
        ("std", "mutex_acquire") => Some("Mutex.acquire"),
        ("std", "mutex_alloc") => Some("Mutex.create"),
        ("std", "mutex_release") => Some("Mutex.release"),
        ("std", "mutex_try_acquire") => Some("Mutex.tryAcquire"),

        // Socket functions
        ("std", "socket_accept") => Some("Socket.accept"),
        ("std", "socket_bind") => Some("Socket.bind"),
        ("std", "socket_close") => Some("Socket.close"),
        ("std", "socket_connect") => Some("Socket.connect"),
        ("std", "socket_fd_size") => Some("Socket.fdSize"),
        ("std", "socket_host") => Some("Socket.host"),
        ("std", "socket_init") => Some("Socket.init"),
        ("std", "socket_listen") => Some("Socket.listen"),
        ("std", "socket_new") => Some("Socket.create"),
        ("std", "socket_peer") => Some("Socket.peer"),
        ("std", "socket_recv") => Some("Socket.recv"),
        ("std", "socket_recv_char") => Some("Socket.recvChar"),
        ("std", "socket_select") => Some("Socket.select"),
        ("std", "socket_send") => Some("Socket.send"),
        ("std", "socket_send_char") => Some("Socket.sendChar"),
        ("std", "socket_set_blocking") => Some("Socket.setBlocking"),
        ("std", "socket_set_fast_send") => Some("Socket.setFastSend"),
        ("std", "socket_set_timeout") => Some("Socket.setTimeout"),
        ("std", "socket_shutdown") => Some("Socket.shutdown"),

        // Date functions
        ("std", "date_from_string") => Some("Date.fromString"),
        ("std", "date_from_time") => Some("Date.fromTime"),
        ("std", "date_get_inf") => Some("Date.getInfo"),
        ("std", "date_get_time") => Some("Date.getTime"),
        ("std", "date_new") => Some("Date.new"),
        ("std", "date_now") => Some("Date.now"),
        ("std", "date_to_string") => Some("Date.toString"),

        // GC functions
        ("std", "gc_dump_memory") => Some("Gc.dumpMemory"),
        ("std", "gc_enable") => Some("Gc.enable"),
        ("std", "gc_major") => Some("Gc.runGC"),

        // NativeArray functions
        ("std", "array_blit") => Some("NativeArray.blit"),
        ("std", "array_type") => Some("NativeArray.getType"),
        ("std", "alloc_array") => Some("NativeArray.alloc"),

        // Host functions
        ("std", "host_local") => Some("Host.localhost"),
        ("std", "host_resolve") => Some("Host.resolve"),
        ("std", "host_reverse") => Some("Host.reverse"),
        ("std", "host_to_string") => Some("Host.toString"),

        // Deque functions
        ("std", "deque_alloc") => Some("Deque.create"),
        ("std", "deque_pop") => Some("Deque.pop"),

        // Hash map functions (hl.types internal)
        ("std", "hballoc") => Some("BytesMap.alloc"),
        ("std", "hbexists") => Some("BytesMap.exists"),
        ("std", "hbget") => Some("BytesMap.get"),
        ("std", "hbkeys") => Some("BytesMap.keys"),
        ("std", "hbremove") => Some("BytesMap.remove"),
        ("std", "hbset") => Some("BytesMap.set"),
        ("std", "hbvalues") => Some("BytesMap.values"),
        ("std", "hialloc") => Some("IntMap.alloc"),
        ("std", "hiexists") => Some("IntMap.exists"),
        ("std", "higet") => Some("IntMap.get"),
        ("std", "hikeys") => Some("IntMap.keys"),
        ("std", "hiremove") => Some("IntMap.remove"),
        ("std", "hiset") => Some("IntMap.set"),
        ("std", "hivalues") => Some("IntMap.values"),
        ("std", "hoalloc") => Some("ObjectMap.alloc"),
        ("std", "hoexists") => Some("ObjectMap.exists"),
        ("std", "hoget") => Some("ObjectMap.get"),
        ("std", "hokeys") => Some("ObjectMap.keys"),
        ("std", "horemove") => Some("ObjectMap.remove"),
        ("std", "hoset") => Some("ObjectMap.set"),
        ("std", "hovalues") => Some("ObjectMap.values"),

        // String/value functions
        ("std", "string_compare") => Some("String.compare"),
        ("std", "value_to_string") => Some("Std.string"),
        ("std", "value_cast") => Some("Std.downcast"),
        ("std", "dyn_compare") => Some("Reflect.compare"),
        ("std", "ptr_compare") => Some("Reflect.compareMethods"),
        ("std", "fun_compare") => Some("Reflect.compareMethods"),

        // Exception functions
        ("std", "exception_stack") => Some("CallStack.exceptionStack"),
        ("std", "set_error_handler") => Some("Api.setErrorHandler"),

        // Internal/low-level functions (keep raw or use common equivalents)
        ("std", "itos") => Some("Std.string"),
        ("std", "ftos") => Some("Std.string"),
        ("std", "no_closure") => Some("Api.noClosure"),
        ("std", "get_closure_value") => Some("Api.getClosureValue"),
        ("std", "get_virtual_value") => Some("Api.getVirtualValue"),
        ("std", "make_closure") => Some("Api.makeClosure"),
        ("std", "make_var_args") => Some("Api.makeVarArgs"),
        ("std", "breakpoint") => Some("Api.breakpoint"),
        ("std", "enum_parameters") => Some("Type.enumParameters"),

        // Process functions
        ("std", "process_close") => Some("Process.close"),
        ("std", "process_exit") => Some("Process.exitCode"),
        ("std", "process_kill") => Some("Process.kill"),
        ("std", "process_pid") => Some("Process.getPid"),
        ("std", "process_run") => Some("Process.run"),
        ("std", "process_stderr_read") => Some("Process.stderrRead"),
        ("std", "process_stdin_close") => Some("Process.stdinClose"),
        ("std", "process_stdin_write") => Some("Process.stdinWrite"),
        ("std", "process_stdout_read") => Some("Process.stdoutRead"),

        // UV functions
        ("uv", "default_loop") => Some("Loop.defaultLoop"),
        ("uv", "loop_alive") => Some("Loop.alive"),
        ("uv", "loop_close") => Some("Loop.close"),
        ("uv", "run") => Some("Loop.run"),
        ("uv", "stop") => Some("Loop.stop"),
        ("uv", "resolve") => Some("Dns.resolve"),

        // SSL functions
        ("ssl", "ssl_init") => Some("Ssl.init"),
        ("ssl", "ssl_new") => Some("Ssl.new"),
        ("ssl", "ssl_close") => Some("Ssl.close"),
        ("ssl", "ssl_handshake") => Some("Ssl.handshake"),
        ("ssl", "ssl_recv") => Some("Ssl.recv"),
        ("ssl", "ssl_recv_char") => Some("Ssl.recvChar"),
        ("ssl", "ssl_send") => Some("Ssl.send"),
        ("ssl", "ssl_send_char") => Some("Ssl.sendChar"),
        ("ssl", "ssl_set_hostname") => Some("Ssl.setHostname"),
        ("ssl", "ssl_set_socket") => Some("Ssl.setSocket"),
        ("ssl", "cert_load_defaults") => Some("Certificate.loadDefaults"),
        ("ssl", "cert_load_file") => Some("Certificate.loadFile"),
        ("ssl", "cert_load_path") => Some("Certificate.loadPath"),
        ("ssl", "conf_new") => Some("Config.new"),
        ("ssl", "conf_close") => Some("Config.close"),
        ("ssl", "conf_set_ca") => Some("Config.setCa"),
        ("ssl", "conf_set_cert") => Some("Config.setCert"),
        ("ssl", "conf_set_verify") => Some("Config.setVerify"),
        ("ssl", "dgst_make") => Some("Digest.make"),

        // Format library
        ("fmt", "inflate_init") => Some("Inflate.init"),
        ("fmt", "inflate_buffer") => Some("Inflate.run"),
        ("fmt", "deflate_init") => Some("Deflate.init"),
        ("fmt", "deflate_buffer") => Some("Deflate.run"),
        ("fmt", "deflate_bound") => Some("Deflate.bound"),
        ("fmt", "png_decode") => Some("Png.decode"),
        ("fmt", "jpg_decode") => Some("Jpg.decode"),
        ("fmt", "ogg_open") => Some("Ogg.open"),
        ("fmt", "ogg_info") => Some("Ogg.info"),
        ("fmt", "ogg_read") => Some("Ogg.read"),
        ("fmt", "ogg_seek") => Some("Ogg.seek"),
        ("fmt", "digest") => Some("Digest.compute"),
        ("fmt", "zip_flush_mode") => Some("Zip.flushMode"),
        ("fmt", "zip_end") => Some("Zip.end"),

        // UI library
        ("ui", "ui_init") => Some("UI.init"),
        ("ui", "ui_loop") => Some("UI.loop"),
        ("ui", "ui_stop_loop") => Some("UI.stopLoop"),
        ("ui", "ui_dialog") => Some("UI.dialog"),
        ("ui", "ui_close_console") => Some("UI.closeConsole"),

        _ => None,
    }
}
