use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/execl_shim.c");
    // cc's default metadata prints a plain `-l static=execl_shim`; we need the
    // whole-archive form below, so suppress cc's own link directives.
    cc::Build::new()
        .file("src/execl_shim.c")
        .cargo_metadata(false)
        .compile("execl_shim");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static:+whole-archive=execl_shim");
    // rustc's generated version script hides every non-Rust symbol behind
    // `local: *;`, which would strip the C hooks below from .dynsym. A second
    // script re-promotes them (scripts merge; globals win over locals).
    let map = out.join("execfilter_exports.map");
    std::fs::write(&map, "{\n  global:\n    execl;\n    execlp;\n};\n").unwrap();
    println!(
        "cargo:rustc-link-arg=-Wl,--version-script={}",
        map.display()
    );
}
