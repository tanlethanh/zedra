use std::env;

fn main() {
    let target = env::var("TARGET").unwrap();

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

        println!("cargo:rustc-link-search=native={}/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/{}",
                 ndk_home, target_arch);
    }
}
