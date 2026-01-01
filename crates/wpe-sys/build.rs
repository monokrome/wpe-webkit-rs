use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let glib = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("glib-2.0")
        .expect("glib-2.0 not found. Install glib2 package.");

    let gobject = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("gobject-2.0")
        .expect("gobject-2.0 not found. Install glib2 package.");

    let wpe_webkit = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("wpe-webkit-2.0")
        .expect("wpe-webkit-2.0 not found. Install wpewebkit package.");

    // WPE Platform API (new, cleaner API)
    let wpe_platform = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("wpe-platform-2.0")
        .expect("wpe-platform-2.0 not found. Install wpewebkit package.");

    // Headless platform for offscreen rendering
    let wpe_platform_headless = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("wpe-platform-headless-2.0")
        .expect("wpe-platform-headless-2.0 not found. Install wpewebkit package.");

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // WPE Platform API
        .allowlist_function("wpe_display_.*")
        .allowlist_function("wpe_view_.*")
        .allowlist_function("wpe_buffer_.*")
        .allowlist_function("wpe_toplevel_.*")
        .allowlist_function("wpe_event_.*")
        .allowlist_function("wpe_rectangle_.*")
        // WebKit API
        .allowlist_function("webkit_.*")
        // JavaScriptCore
        .allowlist_function("jsc_.*")
        // GLib/GObject
        .allowlist_function("g_object_ref")
        .allowlist_function("g_object_unref")
        .allowlist_function("g_object_new")
        .allowlist_function("g_signal_connect_data")
        .allowlist_function("g_signal_emit_by_name")
        .allowlist_function("g_main_context_.*")
        .allowlist_function("g_main_loop_.*")
        .allowlist_function("g_bytes_.*")
        .allowlist_function("g_error_free")
        .allowlist_function("g_free")
        // Types
        .allowlist_type("WPE.*")
        .allowlist_type("WebKit.*")
        .allowlist_type("JSC.*")
        .allowlist_type("GObject")
        .allowlist_type("GBytes")
        .allowlist_type("GError")
        .allowlist_type("GSignal.*")
        .allowlist_type("GMainContext")
        .allowlist_type("GMainLoop")
        .allowlist_var("WPE_.*")
        .allowlist_var("WEBKIT_.*")
        .generate_comments(true)
        .derive_debug(true)
        .derive_default(true);

    let all_include_paths = glib
        .include_paths
        .iter()
        .chain(gobject.include_paths.iter())
        .chain(wpe_webkit.include_paths.iter())
        .chain(wpe_platform.include_paths.iter())
        .chain(wpe_platform_headless.include_paths.iter());

    for path in all_include_paths {
        builder = builder.clang_arg(format!("-I{}", path.display()));
    }

    let bindings = builder
        .generate()
        .expect("Failed to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Failed to write bindings");
}
