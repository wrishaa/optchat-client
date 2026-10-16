//! Android Keystore-backed tokens. Java handles encryption; Rust never writes plaintext tokens.
use gpui_mobile::android::jni as bridge;
use jni::objects::{JObject, JValue};
use optchat_platform::{Platform, SecureStore};
use std::sync::Arc;

pub struct Android;
struct Tokens;

#[no_mangle]
pub extern "system" fn Java_com_reframe_optchat_gpui_android_OptChatActivity_nativeBack(
    _env: *mut std::ffi::c_void, _activity: *mut std::ffi::c_void,
) { gpui_mobile::android::editor::request_back(); }

#[no_mangle]
pub unsafe extern "system" fn Java_com_reframe_optchat_gpui_android_OptChatActivity_nativeEditor(
    _env: *mut std::ffi::c_void, _activity: *mut std::ffi::c_void,
    session: i64, revision: i64, text: *mut std::ffi::c_void,
    start: i32, end: i32, composing_start: i32, composing_end: i32,
) {
    let _ = bridge::with_env(|env| {
        let value = unsafe { JObject::from_raw(env, text as jni::sys::jobject) };
        gpui_mobile::android::editor::enqueue(gpui_mobile::android::editor::Snapshot {
            session, revision, text: bridge::get_string(env, &value),
            start: start.max(0) as usize, end: end.max(0) as usize,
            composing: (composing_start >= 0 && composing_end >= composing_start)
                .then_some(composing_start as usize..composing_end as usize),
        });
        Ok(())
    });
}

#[no_mangle]
pub extern "system" fn Java_com_reframe_optchat_gpui_android_OptChatActivity_nativeInsets(
    _env: *mut std::ffi::c_void, _activity: *mut std::ffi::c_void,
    top: i32, right: i32, bottom: i32, left: i32,
) {
    gpui_mobile::android::editor::set_insets([top, right, bottom, left]);
}

impl Platform for Android {
    fn secure_store(&self) -> Option<Arc<dyn SecureStore>> { Some(Arc::new(Tokens)) }
    fn prepare_microphone(&self) -> Result<(), String> {
        bridge::with_env(|env| {
            let activity = bridge::activity(env)?;
            let allowed = env.call_method(&activity, jni::jni_str!("prepareMicrophone"), jni::jni_sig!("()Z"), &[])
                .map_err(|e| e.to_string())?.z().map_err(|e| e.to_string())?;
            if allowed { Ok(()) } else {
                Err("Allow microphone access in Android, then try dictation again.".into())
            }
        })
    }
}

impl SecureStore for Tokens {
    fn get(&self, key: &str) -> Option<String> {
        bridge::with_env(|env| {
            let activity = bridge::activity(env)?;
            let key = env.new_string(key).map_err(|e| e.to_string())?;
            let value = env.call_method(&activity, jni::jni_str!("getToken"), jni::jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"),
                &[JValue::Object(key.as_ref())]).map_err(|e| e.to_string())?.l().map_err(|e| e.to_string())?;
            Ok((!value.is_null()).then(|| bridge::get_string(env, &value)))
        }).ok().flatten()
    }

    fn set(&self, key: &str, token: &str) -> Result<(), String> {
        bridge::with_env(|env| {
            let activity = bridge::activity(env)?;
            let key = env.new_string(key).map_err(|e| e.to_string())?;
            let token = env.new_string(token).map_err(|e| e.to_string())?;
            let ok = env.call_method(&activity, jni::jni_str!("setToken"), jni::jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Z"),
                &[JValue::Object(key.as_ref()), JValue::Object(token.as_ref())])
                .map_err(|e| e.to_string())?.z().map_err(|e| e.to_string())?;
            if ok { Ok(()) } else { Err("Android secure token storage failed".into()) }
        })
    }

    fn delete(&self, key: &str) {
        let _ = bridge::with_env(|env| {
            let activity = bridge::activity(env)?;
            let key = env.new_string(key).map_err(|e| e.to_string())?;
            env.call_method(&activity, jni::jni_str!("deleteToken"), jni::jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(key.as_ref())]).map_err(|e| e.to_string())?;
            Ok(())
        });
    }
}
