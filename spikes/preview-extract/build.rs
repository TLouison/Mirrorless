fn main() {
    cc::Build::new().file("csrc/shim.c").compile("mmshim");
    println!("cargo:rerun-if-changed=csrc/shim.c");
    // libraw_r is the thread-safe build; the job system will call it from many threads.
    println!("cargo:rustc-link-lib=raw_r");
    println!("cargo:rustc-link-lib=stdc++");
}
