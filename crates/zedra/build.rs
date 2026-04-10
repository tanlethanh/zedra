use std::env;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();

    if target.contains("android") {
        // Android-specific build configuration
        println!("cargo:rustc-link-lib=log");

        // Set up paths for Android NDK
        let ndk_home = env::var("ANDROID_NDK_HOME")
            .or_else(|_| env::var("NDK_HOME"))
            .expect("ANDROID_NDK_HOME or NDK_HOME must be set");

        let target_arch = if target.contains("aarch64") {
            "arm64-v8a"
        } else if target.contains("armv7") {
            "armeabi-v7a"
        } else if target.contains("i686") {
            "x86"
        } else {
            "x86_64"
        };

        println!(
            "cargo:rustc-link-search=native={}/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/{}",
            ndk_home, target_arch
        );
    }

    if target.contains("apple-ios") {
        // Weak stub for ios_present_qr_scanner — lets the cdylib link succeed.
        // The real ObjC implementation in ZedraQRScanner.m overrides this at Xcode link time.
        println!("cargo:rerun-if-changed=src/ios_stub.c");
        cc::Build::new()
            .file("src/ios_stub.c")
            .flag("-Wno-unused-parameter")
            .compile("ios_stub");

        // NSLog bridge — routes Rust log output through NSLog so it appears
        // in idevicesyslog (os_log goes to the unified log, not ASL relay).
        println!("cargo:rerun-if-changed=src/ios/nslog_bridge.m");
        cc::Build::new()
            .file("src/ios/nslog_bridge.m")
            .flag("-fobjc-arc")
            .compile("nslog_bridge");

        let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        cbindgen::Builder::new()
            .with_crate(crate_dir)
            .with_language(cbindgen::Language::C)
            .with_include_guard("ZEDRA_IOS_H")
            .generate()
            .expect("Unable to generate bindings")
            .write_to_file("../../include/zedra_ios.h");
    }
}
