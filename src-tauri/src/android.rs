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
