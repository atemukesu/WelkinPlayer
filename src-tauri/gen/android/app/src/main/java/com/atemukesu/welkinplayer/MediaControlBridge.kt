package com.atemukesu.welkinplayer

import android.content.Context
import android.content.Intent
import android.os.Build

/**
 * Static entry point for the Rust backend to the system media notification.
 *
 * Rust peers into this object over JNI
 * (`com/atemukesu/welkinplayer/MediaControlBridge`): it starts / stops the
 * foreground [MediaPlaybackService] and forwards metadata / playback snapshots.
 * Transport actions requested by the notification, lock screen or headset
 * buttons are parked here and drained by the main window.
 */
object MediaControlBridge {
    /** Running service, or null when the notification is down. */
    @Volatile
    var service: MediaPlaybackService? = null

    /** True between requesting the service and its `onCreate` attaching. */
    @Volatile
    private var starting: Boolean = false

    /**
     * The first snapshot can arrive before the service is up (Rust starts the
     * service and immediately pushes state). It is kept here — the newest one —
     * and replayed once the service attaches.
     */
    private var pendingState: String? = null

    /** Newest transport action ("play" / "seek" / ...) from the system UI. */
    private var pendingControl: String? = null

    /** Called from the MediaSession / notification actions. */
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

    /** Hand the freshly created service any snapshot queued before it existed. */
    @Synchronized
    fun attach(instance: MediaPlaybackService) {
        service = instance
        starting = false
        pendingState?.let { instance.update(it) }
        pendingState = null
    }

    @Synchronized
    fun detach() {
        service = null
        starting = false
    }

    @JvmStatic
    fun start(context: Context) {
        if (service != null || starting) return
        starting = true
        try {
            val intent = Intent(context, MediaPlaybackService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        } catch (error: Throwable) {
            // Android 12+ forbids starting a foreground service from the
            // background; it will come up next time the app is foreground.
            starting = false
        }
    }

    @JvmStatic
    fun stop(context: Context) {
        context.stopService(Intent(context, MediaPlaybackService::class.java))
    }

    @JvmStatic
    @Synchronized
    fun update(json: String) {
        val instance = service
        if (instance == null) {
            pendingState = json
            return
        }
        instance.update(json)
    }
}
