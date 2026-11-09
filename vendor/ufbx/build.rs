fn main() {
    println!("cargo:rerun-if-changed=ufbx/ufbx.c");
    println!("cargo:rerun-if-changed=ufbx/ufbx.h");
    let mut build = cc::Build::new();
    build.file("ufbx/ufbx.c");
    if build.get_compiler().is_like_msvc() {
        // Use direct CRT calls; the final Rust binary selects its runtime.
        // /Zl avoids injecting a conflicting CRT default library into test DLLs.
        build.static_crt(true).flag("/Zl");
    }
    build.compile("ufbx");
}
