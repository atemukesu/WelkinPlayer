// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Exposes the Android JNI handles the native bridges need.
//!
//! # Why this does not touch `ndk-context`
//!
//! tao publishes the process-wide context with
//! `ndk_context::initialize_android_context` exactly once, from `Rust.onCreate`,
//! and that call runs before `onFirstActivityCreate` starts the Rust entry point.
//! `ndk_context` stores the context in a single slot and asserts the slot was
//! empty, so publishing it a second time aborts the process ("assertion failed:
//! previous.is_none()"). This module therefore only caches the handles it needs
//! for its own JNI calls and leaves `ndk-context` to tao.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use jni::objects::{GlobalRef, JObject, JValue};
use jni::JavaVM;

/// Kotlin bridge that owns the floating overlay service (binary name).
const BRIDGE_CLASS_NAME: &str = "com.atemukesu.welkinplayer.DesktopLyricBridge";

/// Kotlin bridge that owns the system media-notification service (binary name).
const MEDIA_BRIDGE_CLASS_NAME: &str = "com.atemukesu.welkinplayer.MediaControlBridge";

/// Kotlin bridge that owns the Storage Access Framework folder picker.
const FOLDER_PICKER_CLASS_NAME: &str = "com.atemukesu.welkinplayer.FolderPickerBridge";

/// Cached global reference to the bridge class. A plain `FindClass` only reaches
/// the system class loader from an attached worker thread, so the app class is
/// resolved once through the Application's class loader instead.
static BRIDGE_CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// Cached global reference to the media-control bridge class.
static MEDIA_BRIDGE_CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// Cached global reference to the folder-picker bridge class.
static FOLDER_PICKER_CLASS: OnceLock<GlobalRef> = OnceLock::new();

/// Whether the floating overlay service is currently running.
static DESKTOP_LYRIC_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The VM obtained in `JNI_OnLoad`; stays valid for the process lifetime.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// The process-wide `Application`, cached because `ndk-context` is owned by tao.
///
/// Resolved once from `ActivityThread.currentApplication()` and kept as a global
/// reference for the process lifetime, since every bridge call needs a
/// `Context` and a plain local reference would be invalid after the resolving
/// thread returns.
static APPLICATION: OnceLock<GlobalRef> = OnceLock::new();

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

/// Caches the `JavaVM` and `Application` handles used by the bridge calls.
///
/// Must run before anything calls the bridges. tao already published the
/// `ndk-context` slot by this point, so this only fills this module's own caches
/// and never re-publishes it.
pub fn initialize() -> Result<(), String> {
    if APPLICATION.get().is_some() {
        return Ok(());
    }

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

    // The static keeps the global reference alive for the process lifetime, so
    // every bridge call can borrow it from any thread.
    let application = env
        .new_global_ref(application)
        .map_err(|error| format!("failed to create a global reference: {error}"))?;
    let _ = APPLICATION.set(application);

    Ok(())
}

/// Borrows the cached `Application` handle for one JNI call.
///
/// The cached global reference outlives every call, so the handle can be reused
/// directly: no new local reference is created, which matters because the Tauri
/// entry point stays attached to the JVM for the process lifetime and local
/// references created there are only released on detach.
///
/// Returns `Err` instead of panicking when [`initialize`] has not run, so a
/// missing context surfaces as a command error rather than an abort.
fn application<'local>() -> Result<JObject<'local>, String> {
    let application = APPLICATION
        .get()
        .ok_or_else(|| "Android application context is not initialized".to_string())?;
    Ok(unsafe { JObject::from_raw(application.as_obj().as_raw()) })
}

/// Borrows the cached `JavaVM` for the playback engine's JNI device queries.
///
/// Returns `None` (rather than panicking) when `JNI_OnLoad` has not captured the
/// VM yet, so a missing handle degrades to "no device list" instead of aborting.
pub fn java_vm<'local>() -> Option<&'static JavaVM> {
    JAVA_VM.get()
}

/// Borrows the cached `Application` handle for the playback engine's JNI calls.
///
/// Mirrors [`application`] but is exposed for cross-module use (the playback
/// worker needs a `Context` to reach `AudioManager`). Returns `None` when the
/// context is not initialized.
pub fn application_jobject<'local>() -> Option<JObject<'local>> {
    application().ok()
}

/// Route audio to a specific output device on Android via `AudioManager`.
///
/// `type_` is the `AudioDeviceInfo.getType()` value (2 = speaker, 3/4 = wired,
/// 8 = Bluetooth A2DP, 11/22 = USB). `name` is the product name used only for
/// logging; the route is driven by the type. Returns `Ok(true)` when the route
/// was applied, `Ok(false)` when the device type is not one we route, and
/// `Err` on a JNI failure.
///
/// We deliberately do *not* reopen the output stream here: cpal's Android
/// backend always follows the system default, so reopening it would not change
/// which device the OS routes to. The stream stays open; only the route changes.
pub fn route_audio(type_: i32, name: &str) -> Result<bool, String> {
    use jni::objects::{JObject, JValue};

    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| format!("failed to attach to the JVM: {error}"))?;
    let _ = env.exception_clear();

    let context = application()?;

    // Context.AUDIO_SERVICE == "audio"
    let service_name = JObject::from(env.new_string("audio").map_err(|error| error.to_string())?);
    let audio_manager = env
        .call_method(
            &context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service_name)],
        )
        .map_err(|error| format!("getSystemService(audio) failed: {error}"))?
        .l()
        .map_err(|error| format!("unexpected audio manager return value: {error}"))?;

    // Route by device type. `setDevicesForMode` is the public routing API
    // (Android 21+); it handles every output device type uniformly, including
    // the built-in speaker, so there is no legacy fallback needed.
    let devices_obj = env
        .call_method(
            &audio_manager,
            "getDevices",
            "(I)[Landroid/media/AudioDeviceInfo;",
            &[JValue::Int(2)],
        )
        .map_err(|error| format!("AudioManager.getDevices failed: {error}"))?
        .l()
        .map_err(|error| format!("unexpected getDevices return value: {error}"))?;
    let devices_array = unsafe { jni::objects::JObjectArray::from_raw(devices_obj.as_raw()) };
    let len = env
        .get_array_length(&devices_array)
        .map_err(|error| format!("get_array_length failed: {error}"))?;
    let mut routed = false;
    for index in 0..len {
        let device = env
            .get_object_array_element(&devices_array, index)
            .map_err(|error| format!("get_object_array_element failed: {error}"))?;
        let dev_type = env
            .call_method(&device, "getType", "()I", &[])
            .map_err(|error| format!("getType failed: {error}"))?
            .i()
            .map_err(|error| format!("unexpected getType return value: {error}"))?;
        if dev_type != type_ {
            continue;
        }
        // setDevicesForMode(mode=0 (NORMAL), [device]) — API 21+.
        // `routeToDevices` does not exist on any Android version; this is the
        // correct public routing API.
        let arr = env
            .new_object_array(
                1,
                "android/media/AudioDeviceInfo",
                JObject::null(),
            )
            .map_err(|error| format!("new_object_array failed: {error}"))?;
        env.set_object_array_element(&arr, 0, &device)
            .map_err(|error| format!("set_object_array_element failed: {error}"))?;
        env.call_method(
            &audio_manager,
            "setDevicesForMode",
            "(I[Landroid/media/AudioDeviceInfo;)V",
            &[JValue::Int(0), JValue::Object(&arr)],
        )
        .map_err(|error| format!("setDevicesForMode failed: {error}"))?;
        routed = true;
        break;
    }
    if !routed {
        return Err(format!(
            "no output device of type {type_} found to route to (name={name})"
        ));
    }
    log::info!("playback: routed to device type={type_} name={name}");
    Ok(true)
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

/// Resolve a Kotlin bridge class through the application class loader.
///
/// `FindClass` on an attached worker thread only reaches the system class
/// loader, which cannot see app classes, so the class is loaded once via the
/// `Application`'s loader and cached as a global reference.
fn class_through_loader<'local>(
    env: &mut jni::JNIEnv<'local>,
    name: &str,
    cell: &'static OnceLock<GlobalRef>,
) -> Result<&'static GlobalRef, String> {
    if let Some(class) = cell.get() {
        return Ok(class);
    }
    let context = application()?;
    let loader = env
        .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    let name = JObject::from(env.new_string(name).map_err(|error| error.to_string())?);
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&name)],
        )
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    let global = env
        .new_global_ref(class)
        .map_err(|error| error.to_string())?;
    let _ = cell.set(global);
    cell.get()
        .ok_or_else(|| "bridge class unavailable".to_string())
}

fn bridge_class<'local>(env: &mut jni::JNIEnv<'local>) -> Result<&'static GlobalRef, String> {
    class_through_loader(env, BRIDGE_CLASS_NAME, &BRIDGE_CLASS)
}

fn media_bridge_class<'local>(env: &mut jni::JNIEnv<'local>) -> Result<&'static GlobalRef, String> {
    class_through_loader(env, MEDIA_BRIDGE_CLASS_NAME, &MEDIA_BRIDGE_CLASS)
}

fn folder_picker_class<'local>(
    env: &mut jni::JNIEnv<'local>,
) -> Result<&'static GlobalRef, String> {
    class_through_loader(env, FOLDER_PICKER_CLASS_NAME, &FOLDER_PICKER_CLASS)
}

fn overlay_permission() -> Result<bool, String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;

    let class = bridge_class(&mut env)?;
    env.call_static_method(
        class,
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
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;

    let class = bridge_class(&mut env)?;
    env.call_static_method(
        class,
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
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;
    let html = JObject::from(env.new_string(html).map_err(|error| error.to_string())?);
    let js = JObject::from(env.new_string(js).map_err(|error| error.to_string())?);

    let class = bridge_class(&mut env)?;
    env.call_static_method(
        class,
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
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;

    let class = bridge_class(&mut env)?;
    env.call_static_method(
        class,
        "stop",
        "(Landroid/content/Context;)V",
        &[JValue::Object(&context)],
    )
    .map_err(|error| error.to_string())?;
    DESKTOP_LYRIC_ACTIVE.store(false, Ordering::SeqCst);
    Ok(())
}

/// Drain a playback action queued by the overlay WebView, if any.
pub fn desktop_lyric_take_control() -> Result<Option<String>, String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let class = bridge_class(&mut env)?;
    let value = env
        .call_static_method(class, "takeControl", "()Ljava/lang/String;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    if value.is_null() {
        return Ok(None);
    }
    let text: String = env
        .get_string(&jni::objects::JString::from(value))
        .map_err(|error| error.to_string())?
        .into();
    Ok(Some(text))
}

/// Forward one document (`load` / `tick` / `settings`) to the overlay WebView.
pub fn desktop_lyric_update(method: &str, json: &str) -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let method = JObject::from(env.new_string(method).map_err(|error| error.to_string())?);
    let json = JObject::from(env.new_string(json).map_err(|error| error.to_string())?);

    let class = bridge_class(&mut env)?;
    env.call_static_method(
        class,
        "update",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::Object(&method), JValue::Object(&json)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Start (or refresh) the foreground media-notification service.
pub fn media_control_start() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;
    let class = media_bridge_class(&mut env)?;
    env.call_static_method(
        class,
        "start",
        "(Landroid/content/Context;)V",
        &[JValue::Object(&context)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Push a metadata/playback snapshot to the native MediaSession.
pub fn media_control_update(json: &str) -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let json = JObject::from(env.new_string(json).map_err(|error| error.to_string())?);
    let class = media_bridge_class(&mut env)?;
    env.call_static_method(
        class,
        "update",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&json)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Update only the native MediaSession playhead, without rebuilding metadata.
///
/// Used on a slow cadence (and immediately after a track loops or is seeked) so
/// the lock screen / notification position does not drift: the platform
/// extrapolates position from the last `setPlaybackState`, so a loop that never
/// reports the reset would keep counting past the track's duration.
pub fn media_control_position(position_ms: f64, playing: bool) -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let class = media_bridge_class(&mut env)?;
    env.call_static_method(
        class,
        "updatePosition",
        "(JZ)V",
        &[
            JValue::Long(position_ms.max(0.0) as i64),
            JValue::Bool(u8::from(playing)),
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Stop the media-notification service and clear its MediaSession.
pub fn media_control_stop() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;
    let class = media_bridge_class(&mut env)?;
    env.call_static_method(
        class,
        "stop",
        "(Landroid/content/Context;)V",
        &[JValue::Object(&context)],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Drain a transport action queued by the native MediaSession, if any.
pub fn media_control_take_control() -> Result<Option<String>, String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let class = media_bridge_class(&mut env)?;
    let value = env
        .call_static_method(class, "takeControl", "()Ljava/lang/String;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    if value.is_null() {
        return Ok(None);
    }
    let text: String = env
        .get_string(&jni::objects::JString::from(value))
        .map_err(|error| error.to_string())?
        .into();
    Ok(Some(text))
}

/// Open the system folder picker (Storage Access Framework).
///
/// Returns the `FolderPickerBridge.PICK_*` status: `0` = picker launched (the
/// result is drained with [`take_folder_pick`]), `1` = the "all files access"
/// settings screen was opened and the user must retry, `-1` = unavailable.
pub fn pick_folder() -> Result<i32, String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let _ = env.exception_clear();
    let class = folder_picker_class(&mut env)?;
    env.call_static_method(class, "pick", "()I", &[])
        .map_err(|error| error.to_string())?
        .i()
        .map_err(|error| error.to_string())
}

/// Drain a finished folder pick.
///
/// `None` = still waiting; `Some(None)` = cancelled; `Some(Some(path))` = the
/// chosen directory. The empty-string sentinel distinguishes "cancelled" from
/// "not ready" because the JNI call returns a null `String` while pending.
pub fn take_folder_pick() -> Result<Option<Option<String>>, String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let _ = env.exception_clear();
    let class = folder_picker_class(&mut env)?;
    let value = env
        .call_static_method(class, "takeResult", "()Ljava/lang/String;", &[])
        .map_err(|error| error.to_string())?
        .l()
        .map_err(|error| error.to_string())?;
    if value.is_null() {
        return Ok(None);
    }
    let text: String = env
        .get_string(&jni::objects::JString::from(value))
        .map_err(|error| error.to_string())?
        .into();
    if text.is_empty() {
        Ok(Some(None))
    } else {
        Ok(Some(Some(text)))
    }
}

/// Whether local-folder access is currently granted.
///
/// A failure reports `true` so a broken JNI context cannot permanently hide the
/// hint; the real error surfaces when the source is read.
pub fn folder_access_granted() -> bool {
    let Some(java_vm) = JAVA_VM.get() else {
        return true;
    };
    let Ok(mut env) = java_vm.attach_current_thread() else {
        return true;
    };
    let _ = env.exception_clear();
    let Ok(class) = folder_picker_class(&mut env) else {
        return true;
    };
    match env
        .call_static_method(class, "hasAccess", "()Z", &[])
        .and_then(|value| value.z())
    {
        Ok(granted) => granted,
        Err(_) => true,
    }
}

/// Open the system screen where local file access is granted.
pub fn request_folder_access() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    let _ = env.exception_clear();
    let class = folder_picker_class(&mut env)?;
    env.call_static_method(class, "requestAccess", "()V", &[])
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Classify the active Android network via `ConnectivityManager`.
///
/// Requires `ACCESS_NETWORK_STATE`; any failure falls back to "unmetered" so a
/// missing permission cannot silently disable streaming.
pub fn network_status() -> crate::core::network::NetworkStatus {
    match query_network() {
        Ok(status) => status,
        Err(error) => {
            log::warn!("network status query failed: {error}");
            crate::core::network::NetworkStatus::UNMETERED
        }
    }
}

fn query_network() -> Result<crate::core::network::NetworkStatus, String> {
    use jni::objects::JValue;

    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let mut env = java_vm
        .attach_current_thread()
        .map_err(|error| error.to_string())?;
    // A failed JNI call leaves its Java exception pending. On an already-attached
    // thread (the Android main thread) the attach guard does not detach, so the
    // stale exception would poison every later call; clear it defensively.
    let _ = env.exception_clear();
    let context = application()?;

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

    Ok(crate::core::network::NetworkStatus {
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

    if let Err(error) = install_keychain_builder() {
        log::error!("failed to initialize android-keyring credential store: {error}");
        return Some(format!("android-keyring 初始化失败：{error}"));
    }

    None
}

/// Install android-keyring's Keystore-backed credential builder.
///
/// The builder is built from this module's cached `Application` instead of
/// `android_keyring::set_android_keyring_credential_builder`, which reads the
/// `ndk-context` slot directly: keeping every JNI handle in one place means the
/// keychain never depends on who published that slot.
fn install_keychain_builder() -> Result<(), String> {
    let java_vm = JAVA_VM
        .get()
        .ok_or_else(|| "JNI_OnLoad has not run yet".to_string())?;
    let env = java_vm
        .attach_current_thread()
        .map_err(|error| format!("failed to attach to the JVM: {error}"))?;
    let _ = env.exception_clear();

    let context = application()?;
    let context = android_keyring::shared_preferences::Context::new(&env, context)
        .map_err(|error| format!("failed to wrap the application context: {error}"))?;
    let builder = android_keyring::credential::AndroidBuilder::new(&env, context)
        .map_err(|error| format!("failed to build the Keystore credential builder: {error}"))?;
    keyring::set_default_credential_builder(Box::new(builder));

    Ok(())
}
