fn main() {
    println!("cargo:rustc-check-cfg=cfg(rtk_library)");
    println!("cargo:rustc-cfg=rtk_library");
}
