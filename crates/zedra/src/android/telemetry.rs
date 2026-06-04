//! Android Firebase Analytics + Crashlytics bridge.

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JObjectArray},
    sys::jint,
};

const FIREBASE_CLASS: &str = "dev/zedra/app/ZedraFirebase";

pub fn log_event(name: &str, params: &[(&str, &str)]) {
    let name = name.to_string();
    let keys: Vec<String> = params.iter().map(|(key, _)| (*key).to_string()).collect();
    let values: Vec<String> = params
        .iter()
        .map(|(_, value)| (*value).to_string())
        .collect();

    super::jni::jni_call("firebase_log_event", move || {
        with_firebase_class("firebase_log_event", |env, class| {
            let name = match env.new_string(&name) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: firebase event name failed");
                    return;
                }
            };
            let keys = match string_array(env, &keys, "firebase event keys") {
                Some(value) => value,
                None => return,
            };
            let values = match string_array(env, &values, "firebase event values") {
                Some(value) => value,
                None => return,
            };

            if let Err(error) = env.call_static_method(
                class,
                "logEvent",
                "(Ljava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V",
                &[(&name).into(), (&keys).into(), (&values).into()],
            ) {
                tracing::error!(?error, "jni: Firebase logEvent failed");
            }
        });
    });
}

pub fn record_error(message: &str, file: &str, line: u32) {
    let message = message.to_string();
    let file = file.to_string();
    super::jni::jni_call("firebase_record_error", move || {
        with_firebase_class("firebase_record_error", |env, class| {
            let message = match env.new_string(&message) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: firebase error message failed");
                    return;
                }
            };
            let file = match env.new_string(&file) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: firebase error file failed");
                    return;
                }
            };

            if let Err(error) = env.call_static_method(
                class,
                "recordError",
                "(Ljava/lang/String;Ljava/lang/String;I)V",
                &[(&message).into(), (&file).into(), (line as jint).into()],
            ) {
                tracing::error!(?error, "jni: Firebase recordError failed");
            }
        });
    });
}

pub fn record_panic(message: &str, location: &str) {
    let message = message.to_string();
    let location = location.to_string();
    super::jni::jni_call("firebase_record_panic", move || {
        call_two_strings("firebase_record_panic", "recordPanic", &message, &location);
    });
}

pub fn set_user_id(id: &str) {
    let id = id.to_string();
    super::jni::jni_call("firebase_set_user_id", move || {
        with_firebase_class("firebase_set_user_id", |env, class| {
            let id = match env.new_string(&id) {
                Ok(value) => value,
                Err(error) => {
                    tracing::error!(?error, "jni: firebase user id failed");
                    return;
                }
            };
            if let Err(error) =
                env.call_static_method(class, "setUserId", "(Ljava/lang/String;)V", &[(&id).into()])
            {
                tracing::error!(?error, "jni: Firebase setUserId failed");
            }
        });
    });
}

pub fn set_custom_key(key: &str, value: &str) {
    let key = key.to_string();
    let value = value.to_string();
    super::jni::jni_call("firebase_set_custom_key", move || {
        call_two_strings("firebase_set_custom_key", "setCustomKey", &key, &value);
    });
}

pub fn set_collection_enabled(enabled: bool) {
    super::jni::jni_call("firebase_set_collection_enabled", move || {
        with_firebase_class("firebase_set_collection_enabled", |env, class| {
            if let Err(error) =
                env.call_static_method(class, "setCollectionEnabled", "(Z)V", &[enabled.into()])
            {
                tracing::error!(?error, "jni: Firebase setCollectionEnabled failed");
            }
        });
    });
}

fn call_two_strings(name: &'static str, method: &'static str, first: &str, second: &str) {
    with_firebase_class(name, |env, class| {
        let first = match env.new_string(first) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(?error, "jni: firebase first string failed");
                return;
            }
        };
        let second = match env.new_string(second) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(?error, "jni: firebase second string failed");
                return;
            }
        };

        if let Err(error) = env.call_static_method(
            class,
            method,
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[(&first).into(), (&second).into()],
        ) {
            tracing::error!(method, ?error, "jni: Firebase string call failed");
        }
    });
}

fn with_firebase_class(
    name: &'static str,
    f: impl for<'local> FnOnce(&mut JNIEnv<'local>, &JClass<'local>),
) {
    super::jni::with_class(name, FIREBASE_CLASS, f);
}

fn string_array<'local>(
    env: &mut JNIEnv<'local>,
    values: &[String],
    label: &'static str,
) -> Option<JObjectArray<'local>> {
    let string_class = match env.find_class("java/lang/String") {
        Ok(class) => class,
        Err(error) => {
            tracing::error!(?error, label, "jni: find String class failed");
            return None;
        }
    };
    let array = match env.new_object_array(values.len() as i32, string_class, JObject::null()) {
        Ok(array) => array,
        Err(error) => {
            tracing::error!(?error, label, "jni: string array failed");
            return None;
        }
    };

    for (index, value) in values.iter().enumerate() {
        let value = match env.new_string(value) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(?error, label, "jni: string array value failed");
                return None;
            }
        };
        if let Err(error) = env.set_object_array_element(&array, index as i32, &value) {
            tracing::error!(?error, label, "jni: populate string array failed");
            return None;
        }
    }

    Some(array)
}
