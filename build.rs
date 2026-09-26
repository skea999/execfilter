fn main() {
    println!("cargo:rerun-if-changed=src/execl_shim.c");
    cc::Build::new().file("src/execl_shim.c").compile("execl_shim");
}
