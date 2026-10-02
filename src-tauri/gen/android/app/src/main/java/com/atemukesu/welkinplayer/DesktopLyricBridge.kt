package com.atemukesu.welkinplayer

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.util.Log
import android.provider.Settings

/**
 * Static entry point for the Rust backend.
 *
 * Rust peers into this object over JNI (`com/atemukesu/welkinplayer/DesktopLyricBridge`):
 * it queries the overlay permission, opens the system settings screen, starts /
 * stops the foreground service and forwards the lyric documents. The HTML
 * renderer is handed over once and kept here so the service can load it.
 */
object DesktopLyricBridge {
    /** Self-contained renderer markup, mirrored by `public/desktop-lyric-overlay.html`. */
    @Volatile
    var html: String = ""

    /** Renderer script, mirrored by `public/desktop-lyric-overlay.js`. */
    @Volatile
    var js: String = ""

    /** Running service, or null when the overlay is down. */
    @Volatile
    var service: DesktopLyricService? = null

    /**
     * The first `load` / `settings` documents can arrive before the service is
     * up (the main window pushes them right after `start()`). They are kept
     * here — the newest of each — and replayed once the service attaches.
     * `tick` documents are intentionally dropped: they are transient and the
     * steady stream resumes immediately.
     */
    private var pendingLoad: String? = null
    private var pendingSettings: String? = null

    /** Newest playback action ("toggle" / "previous" / "next") from the overlay. */
    private var pendingControl: String? = null

    /** Called from the overlay WebView when a media button is tapped. */
    @Synchronized
    fun control(action: String) {
        pendingControl = action
    }

    /**
     * Polled by the Rust backend on a short interval; returns and clears the
     * pending action so the main window can drive the player.
     */
    @Synchronized
    @JvmStatic
    fun takeControl(): String? {
        val value = pendingControl ?: return null
        pendingControl = null
        return value
    }

    /** Hand the freshly created service any documents queued before it existed. */
    @Synchronized
    fun attachService(instance: DesktopLyricService) {
        service = instance
        pendingLoad?.let { instance.update("load", it) }
        pendingSettings?.let { instance.update("settings", it) }
        pendingLoad = null
        pendingSettings = null
    }

    @Synchronized
    fun detachService() {
        service = null
    }

    @JvmStatic
    fun hasOverlayPermission(context: Context): Boolean = Settings.canDrawOverlays(context)

    @JvmStatic
    fun openOverlaySettings(context: Context) {
        val intent = Intent(
            Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
            Uri.parse("package:" + context.packageName)
        )
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(intent)
    }

    @JvmStatic
    fun start(context: Context, rendererHtml: String, rendererJs: String) {
        html = rendererHtml
        js = rendererJs
        Log.i("WelkinLyric", "bridge.start html=${rendererHtml.length} js=${rendererJs.length}")
        try {
            val intent = Intent(context, DesktopLyricService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        } catch (error: Throwable) {
            // Starting a foreground service from the background is restricted on
            // Android 12+; swallow it rather than letting the exception cross JNI
            // and poison the main thread for every later call.
        }
    }

    @JvmStatic
    fun stop(context: Context) {
        context.stopService(Intent(context, DesktopLyricService::class.java))
    }

    @JvmStatic
    @Synchronized
    fun update(method: String, json: String) {
        val instance = service
        if (instance == null) {
            when (method) {
                "load" -> pendingLoad = json
                "settings" -> pendingSettings = json
            }
            return
        }
        instance.update(method, json)
    }
}
