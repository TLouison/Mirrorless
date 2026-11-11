fn main() {
    cc::Build::new().cpp(true).file("csrc/shim.cpp").compile("mmshim");
    println!("cargo:rerun-if-changed=csrc/shim.cpp");
    // libraw_r is the thread-safe build; the job system will call it from many threads.
    println!("cargo:rustc-link-lib=raw_r");
}
