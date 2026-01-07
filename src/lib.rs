use android_logger::Config;
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use log::info;

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_initRust(_env: JNIEnv, _class: JClass) {
    // Initialize Android logger
    android_logger::init_once(
        Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("RustApp"),
    );

    info!("Rust library initialized!");
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_rustGreeting(
    mut env: JNIEnv,
    _class: JClass,
    input: JString,
) -> jstring {
    let input: String = env
        .get_string(&input)
        .expect("Couldn't get java string!")
        .into();

    let output = format!("Hello from Rust, {}!", input);

    let output = env
        .new_string(output)
        .expect("Couldn't create java string!");

    output.into_raw()
}

// Android lifecycle hooks
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnResume(_env: JNIEnv, _class: JClass) {
    info!("App resumed");
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_zedra_app_MainActivity_rustOnPause(_env: JNIEnv, _class: JClass) {
    info!("App paused");
}
