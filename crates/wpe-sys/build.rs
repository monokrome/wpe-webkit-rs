use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let wpe = pkg_config::Config::new()
        .atleast_version("1.0")
        .probe("wpe-1.0")
        .expect("libwpe-1.0 not found. Install libwpe package.");

    let wpe_webkit = pkg_config::Config::new()
        .atleast_version("2.0")
        .probe("wpe-webkit-2.0")
        .expect("wpe-webkit-2.0 not found. Install wpewebkit package.");

    let wpe_fdo = pkg_config::Config::new()
        .atleast_version("1.0")
        .probe("wpebackend-fdo-1.0")
        .expect("wpebackend-fdo-1.0 not found. Install wpebackend-fdo package.");

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_function("wpe_.*")
        .allowlist_function("webkit_.*")
        .allowlist_type("WPE.*")
        .allowlist_type("WebKit.*")
        .allowlist_var("WPE_.*")
        .allowlist_var("WEBKIT_.*")
        .generate_comments(true)
        .derive_debug(true)
        .derive_default(true);

    for path in wpe.include_paths.iter().chain(wpe_webkit.include_paths.iter()).chain(wpe_fdo.include_paths.iter()) {
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
