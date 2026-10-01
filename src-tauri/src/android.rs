//! Publishes the Android JNI context that `ndk-context` consumers depend on.
//!
//! tao 0.35 no longer initializes the `ndk-context` crate, but `android-keyring`
//! (through `ndk-context`) panics when that context is missing. We capture the
//! `JavaVM` in `JNI_OnLoad` and resolve the process-wide `Application`, so the
//! context is ready by the time the Tauri entry point runs on a worker thread.

use std::ffi::c_void;
use std::sync::OnceLock;

use jni::JavaVM;

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
