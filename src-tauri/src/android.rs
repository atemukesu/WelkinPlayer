//! Publishes the Android JNI context that `ndk-context` consumers depend on.
//!
//! tao 0.35 no longer initializes the `ndk-context` crate, but `android-keyring`
//! (through `ndk-context`) panics when that context is missing. We capture the
//! `JavaVM` in `JNI_OnLoad` and resolve the process-wide `Application`, so the
//! context is ready by the time the Tauri entry point runs on a worker thread.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use jni::objects::{JObject, JValue};
use jni::JavaVM;

/// Kotlin bridge that owns the floating overlay service.
const DESKTOP_LYRIC_BRIDGE: &str = "com/atemukesu/welkinplayer/DesktopLyricBridge";

/// Whether the floating overlay service is currently running.
static DESKTOP_LYRIC_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The VM obtained in `JNI_OnLoad`; stays valid for the process lifetime.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// Captures the `JavaVM` as soon as the native library is loaded.
///
/// # Safety
/// The JVM calls this during `System.loadLibrary`, so `vm` is a valid
/// `JavaVM*`. Returning a supported version keeps the JNI contract happy.
#[no_mangle]
pub extern "system" fn JNI_OnLoad(
    vm: *mut jni::sys::JavaVM,
    _reserved: *mut c_void,
) -> jni::sys::jint {
    match unsafe { JavaVM::from_raw(vm) } {
        Ok(java_vm) => {
            let _ = JAVA_VM.set(java_vm);
        }
        Err(error) => log::error!("JNI_OnLoad could not capture the JavaVM: {error}"),
    }
    jni::sys::JNI_VERSION_1_6
}

/// Publishes the JVM and Application object to `ndk-context`.
///
/// Must run before anything reads `ndk_context::android_context()`.
pub fn initialize() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;

    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| format!("failed to attach to the JVM: {error}"))?;

    // currentApplication() is the process-wide Application object, so it works
    // even though this runs before the Activity itself has been created.
    let application = env
        .call_static_method(
            "android/app/ActivityThread",
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        )
        .map_err(|error| format!("failed to resolve the application object: {error}"))?
        .l()
        .map_err(|error| format!("unexpected application return value: {error}"))?;

    if application.as_raw().is_null() {
        return Err("ActivityThread.currentApplication() returned null".to_string());
    }

    // ndk-context stores bare pointers and never frees them, so the global
    // reference must outlive this call; leaking it deliberately keeps it valid.
    let application = env
        .new_global_ref(application)
        .map_err(|error| format!("failed to create a global reference: {error}"))?;

    unsafe {
        ndk_context::initialize_android_context(
            java_vm.get_java_vm_pointer().cast(),
            application.as_obj().as_raw().cast(),
        );
    }
    std::mem::forget(application);

    Ok(())
}

/// The overlay exists on every supported Android version.
pub fn desktop_lyric_supported() -> bool {
    true
}

/// Whether the floating overlay service is currently running.
pub fn desktop_lyric_is_running() -> bool {
    DESKTOP_LYRIC_ACTIVE.load(Ordering::SeqCst)
}

/// Whether the "display over other apps" permission has been granted.
///
/// A failed query reports `true` so a broken JNI context cannot silently block
/// the feature; the real failure surfaces when the service refuses to start.
pub fn desktop_lyric_has_permission() -> bool {
    match overlay_permission() {
        Ok(value) => value,
        Err(error) => {
            log::warn!("overlay permission query failed: {error}");
            true
        }
    }
}

fn overlay_permission() -> Result<bool, String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let context_raw = ndk_context::android_context().context();
    let context = unsafe { JObject::from_raw(context_raw as jni::sys::jobject) };

    env.call_static_method(
        DESKTOP_LYRIC_BRIDGE,
        "hasOverlayPermission",
        "(Landroid/content/Context;)Z",
        &[JValue::Object(&context)],
    )
    .map_err(|error| error.to_string())?
    .z()
    .map_err(|error| error.to_string())
}

/// Open the system screen where the user grants the overlay permission.
pub fn desktop_lyric_open_settings() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let context_raw = ndk_context::android_context().context();
    let context = unsafe { JObject::from_raw(context_raw as jni::sys::jobject) };

    env.call_static_method(
        DESKTOP_LYRIC_BRIDGE,
        "openOverlaySettings",
        "(Landroid/content/Context;)V",
        &[JValue::Object(&context)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Start the foreground overlay service with the bundled renderer markup/script.
pub fn desktop_lyric_start(html: &str, js: &str) -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let context_raw = ndk_context::android_context().context();
    let context = unsafe { JObject::from_raw(context_raw as jni::sys::jobject) };
    let html = JObject::from(env.new_string(html).map_err(|error| error.to_string())?);
    let js = JObject::from(env.new_string(js).map_err(|error| error.to_string())?);

    env.call_static_method(
        DESKTOP_LYRIC_BRIDGE,
        "start",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(&context),
            JValue::Object(&html),
            JValue::Object(&js),
        ],
    )
    .map_err(|error| error.to_string())?;
    DESKTOP_LYRIC_ACTIVE.store(true, Ordering::SeqCst);
    Ok(())
}

/// Stop the foreground overlay service.
pub fn desktop_lyric_stop() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let context_raw = ndk_context::android_context().context();
    let context = unsafe { JObject::from_raw(context_raw as jni::sys::jobject) };

    env.call_static_method(
        DESKTOP_LYRIC_BRIDGE,
        "stop",
        "(Landroid/content/Context;)V",
        &[JValue::Object(&context)],
    )
    .map_err(|error| error.to_string())?;
    DESKTOP_LYRIC_ACTIVE.store(false, Ordering::SeqCst);
    Ok(())
}

/// Forward one document (`load` / `tick` / `settings`) to the overlay WebView.
pub fn desktop_lyric_update(method: &str, json: &str) -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let method = JObject::from(env.new_string(method).map_err(|error| error.to_string())?);
    let json = JObject::from(env.new_string(json).map_err(|error| error.to_string())?);

    env.call_static_method(
        DESKTOP_LYRIC_BRIDGE,
        "update",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::Object(&method), JValue::Object(&json)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Classify the active Android network via `ConnectivityManager`.
///
/// Requires `ACCESS_NETWORK_STATE`; any failure falls back to "unmetered" so a
/// missing permission cannot silently disable streaming.
pub fn network_status() -> crate::network::NetworkStatus {
    match query_network() {
        Ok(status) => status,
        Err(error) => {
            log::warn!("network status query failed: {error}");
            crate::network::NetworkStatus::UNMETERED
        }
    }
}

fn query_network() -> Result<crate::network::NetworkStatus, String> {
    use jni::objects::{JObject, JValue};

    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let context_raw = ndk_context::android_context().context();
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let context = unsafe { JObject::from_raw(context_raw as jni::sys::jobject) };

    let service = env
        .new_string("connectivity")
        .map_err(|error| error.to_string())?;
    let manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    if manager.is_null() {
        return Err("ConnectivityManager unavailable".to_string());
    }

    // `isActiveNetworkMetered()` covers cellular, metered Wi-Fi hotspots, etc.
    let metered = env
        .call_method(&manager, "isActiveNetworkMetered", "()Z", &[])
        .map_err(|error| error.to_string())?
        .z()
        .map_err(|error| error.to_string())?;

    let network = env
        .call_method(&manager, "getActiveNetwork", "()Landroid/net/Network;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    let available = !network.is_null();

    let mut wifi = false;
    if available {
        if let Ok(caps) = env
            .call_method(
                &manager,
                "getNetworkCapabilities",
                "(Landroid/net/Network;)Landroid/net/NetworkCapabilities;",
                &[JValue::Object(&network)],
            )
            .and_then(|value| value.l())
        {
            if !caps.is_null() {
                // NetworkCapabilities.TRANSPORT_WIFI == 1
                wifi = env
                    .call_method(&caps, "hasTransport", "(I)Z", &[JValue::Int(1)])
                    .and_then(|value| value.z())
                    .unwrap_or(false);
            }
        }
    }

    Ok(crate::network::NetworkStatus {
        metered,
        wifi: wifi && !metered,
        available,
    })
}

/// Install the Keystore-backed keyring store, or explain why it cannot be used.
///
/// The `keyring` crate has no Android backend of its own, so when this setup is
/// skipped keyring silently falls back to an in-memory mock: it accepts writes
/// and reports success, but drops the secret as soon as the process exits. The
/// caller records the returned reason so the WebDAV commands fail loudly instead
/// of pretending a credential was saved.
pub fn initialize_keychain() -> Option<String> {
    if let Err(error) = initialize() {
        log::error!("Android JNI context unavailable; system keychain disabled: {error}");
        return Some(format!("Android JNI 上下文不可用：{error}"));
    }

    if let Err(error) = android_keyring::set_android_keyring_credential_builder() {
        log::error!("failed to initialize android-keyring credential store: {error}");
        return Some(format!("android-keyring 初始化失败：{error}"));
    }

    None
}
