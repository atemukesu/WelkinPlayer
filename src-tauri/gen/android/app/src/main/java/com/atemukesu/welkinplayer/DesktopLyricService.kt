package com.atemukesu.welkinplayer

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.webkit.JavascriptInterface
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.core.app.NotificationCompat
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Foreground service hosting the floating lyrics overlay.
 *
 * A [WebView] is added straight to the [WindowManager] as a `TYPE_APPLICATION_OVERLAY`
 * window and runs the same self-contained renderer as the desktop window. The
 * Rust backend pushes documents through [DesktopLyricBridge.update], which lands
 * here and is evaluated inside the page.
 */
class DesktopLyricService : Service() {
    private var windowManager: WindowManager? = null
    private var webView: WebView? = null
    private var params: WindowManager.LayoutParams? = null
    private val main = Handler(Looper.getMainLooper())
    private val pending = ArrayDeque<String>()
    private var pageReady = false
    private var locked = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "onCreate")
        createChannel()
        startForegroundCompat(buildNotification())
        createOverlay()
        // Only now are we able to accept the documents that were queued while
        // the service was starting.
        DesktopLyricBridge.attachService(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i(TAG, "onStartCommand action=${intent?.action}")
        when (intent?.action) {
            ACTION_TOGGLE_LOCK -> {
                val next = !locked
                setLocked(next)
                // Keep the main window's toggle in sync with the notification.
                DesktopLyricBridge.control(if (next) "lock" else "unlock")
                return START_NOT_STICKY
            }
            ACTION_CLOSE -> {
                DesktopLyricBridge.control("close")
                stopSelf()
                return START_NOT_STICKY
            }
        }
        // A plain start (no action): load the renderer once.
        loadRenderer()
        return START_NOT_STICKY
    }

    /** The app's task was swiped away: the lyrics should go with it. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        Log.i(TAG, "onDestroy")
        webView?.let { view ->
            try {
                windowManager?.removeView(view)
            } catch (_: Throwable) {
            }
            view.destroy()
        }
        webView = null
        params = null
        DesktopLyricBridge.detachService()
        super.onDestroy()
    }

    /** Run one renderer document (`load` / `settings` / `tick`) on the UI thread. */
    fun update(method: String, json: String) {
        val script = when (method) {
            "load" -> "window.__welkinOverlay && window.__welkinOverlay.load($json);"
            "settings" -> "window.__welkinOverlay && window.__welkinOverlay.settings($json);"
            "tick" -> "window.__welkinOverlay && window.__welkinOverlay.tick($json);"
            else -> return
        }
        main.post {
            val view = webView ?: return@post
            if (pageReady) view.evaluateJavascript(script, null) else pending.addLast(script)
        }
    }

    private fun setLocked(value: Boolean) {
        locked = value
        main.post {
            val view = webView ?: return@post
            val layout = params ?: return@post
            // Locked means click-through: the full-width overlay would otherwise
            // swallow touches meant for the app behind it. Unlock happens via
            // the notification action (the hover padlock is a desktop affordance).
            layout.flags = if (value) {
                layout.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
            } else {
                layout.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
            }
            try {
                windowManager?.updateViewLayout(view, layout)
            } catch (_: Throwable) {
            }
            updateNotification()
        }
    }

    private fun setVisible(value: Boolean) {
        Log.i(TAG, "setVisible $value")
        main.post {
            webView?.visibility = if (value) View.VISIBLE else View.GONE
        }
    }

    private fun persistPosition(x: Int, y: Int) {
        getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putInt(POSITION_X, x)
            .putInt(POSITION_Y, y)
            .apply()
    }

    private fun closeOverlay() {
        main.post { stopSelf() }
    }

    private fun createOverlay() {
        val manager = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        windowManager = manager

        val view = WebView(this)
        view.setBackgroundColor(Color.TRANSPARENT)
        view.isVerticalScrollBarEnabled = false
        view.isHorizontalScrollBarEnabled = false
        view.settings.javaScriptEnabled = true
        view.settings.domStorageEnabled = true
        view.settings.mediaPlaybackRequiresUserGesture = false
        view.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView?, url: String?) {
                pageReady = true
                val queued = pending.toList()
                pending.clear()
                // The bare WebView cannot resolve the app asset path, so the
                // renderer script is injected here instead of via `<script src>`.
                val renderer = DesktopLyricBridge.js
                val combined = if (queued.isEmpty()) renderer else renderer + "\n" + queued.joinToString("\n")
                Log.i(TAG, "onPageFinished renderer=${renderer.length} queued=${queued.size}")
                if (combined.isNotBlank()) view?.evaluateJavascript(combined, null)
                // Native fallback: reveal the layer as soon as the page is up so
                // the overlay never stays invisible even if the JS handshake is
                // missed.
                setVisible(true)
            }
        }
        view.addJavascriptInterface(BridgeInterface(), "AndroidDesktopLyric")
        view.setOnTouchListener(DragTouchListener())

        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        } else {
            @Suppress("DEPRECATION")
            WindowManager.LayoutParams.TYPE_PHONE
        }
        val layout = WindowManager.LayoutParams(
            WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            type,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS or
                WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
            PixelFormat.TRANSLUCENT
        ).apply {
            gravity = Gravity.TOP or Gravity.START
            x = getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(POSITION_X, 0)
            y = getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(POSITION_Y, (resources.displayMetrics.heightPixels * 0.55f).toInt())
        }

        view.visibility = View.GONE
        try {
            manager.addView(view, layout)
            Log.i(TAG, "overlay added")
        } catch (error: Throwable) {
            // Permission revoked between the check and now, or an incompatible
            // OEM overlay policy: bail out instead of crashing the process.
            Log.e(TAG, "addView failed", error)
            view.destroy()
            stopSelf()
            return
        }
        webView = view
        params = layout
    }

    private fun loadRenderer() {
        val view = webView ?: run { Log.w(TAG, "loadRenderer: no webView"); return }
        val html = DesktopLyricBridge.html
        Log.i(TAG, "loadRenderer html=${html.length}")
        if (html.isBlank()) return
        pageReady = false
        view.loadDataWithBaseURL("https://desktop-lyric.local/", html, "text/html", "utf-8", null)
    }

    /**
     * Drags the overlay with `MotionEvent.rawX/rawY`, which are true screen
     * coordinates. The WebView's own `clientX/Y` are window-relative, so they
     * shrink by exactly as much as the window moves; feeding that back into a
     * `setPosition` loop made the layer accelerate away and fly across the
     * screen. Raw coordinates do not move when the window does, so dragging
     * stays 1:1 with the finger no matter how the position is clamped.
     *
     * The listener never consumes the event: the WebView still needs it for the
     * buttons and the renderer's own gesture bookkeeping.
     */
    private inner class DragTouchListener : View.OnTouchListener {
        private val touchSlop = ViewConfiguration.get(this@DesktopLyricService).scaledTouchSlop
        private var startRawX = 0f
        private var startRawY = 0f
        private var startLayoutX = 0
        private var startLayoutY = 0
        private var dragging = false

        override fun onTouch(view: View, event: MotionEvent): Boolean {
            val layout = params ?: return false
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    startRawX = event.rawX
                    startRawY = event.rawY
                    startLayoutX = layout.x
                    startLayoutY = layout.y
                    dragging = false
                }
                MotionEvent.ACTION_MOVE -> {
                    if (!dragging) {
                        dragging = abs(event.rawX - startRawX) > touchSlop ||
                            abs(event.rawY - startRawY) > touchSlop
                    }
                    if (dragging) {
                        val metrics = resources.displayMetrics
                        val maxX = (metrics.widthPixels - view.width).coerceAtLeast(0)
                        val maxY = (metrics.heightPixels - view.height).coerceAtLeast(0)
                        layout.x = (startLayoutX + (event.rawX - startRawX)).roundToInt().coerceIn(0, maxX)
                        layout.y = (startLayoutY + (event.rawY - startRawY)).roundToInt().coerceIn(0, maxY)
                        try {
                            windowManager?.updateViewLayout(view, layout)
                        } catch (_: Throwable) {
                        }
                    }
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                    if (dragging) persistPosition(layout.x, layout.y)
                    dragging = false
                }
            }
            return false
        }
    }

    /** JS surface the renderer uses to drive the native window. */
    private inner class BridgeInterface {
        @JavascriptInterface
        fun setLocked(value: Boolean) = this@DesktopLyricService.setLocked(value)

        @JavascriptInterface
        fun setVisible(value: Boolean) = this@DesktopLyricService.setVisible(value)

        @JavascriptInterface
        fun close() = this@DesktopLyricService.closeOverlay()

        @JavascriptInterface
        fun control(action: String) = DesktopLyricBridge.control(action)
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Desktop lyrics",
            NotificationManager.IMPORTANCE_LOW
        )
        channel.setShowBadge(false)
        manager.createNotificationChannel(channel)
    }

    private fun startForegroundCompat(notification: Notification) {
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun updateNotification() {
        val manager = getSystemService(NotificationManager::class.java) ?: return
        manager.notify(NOTIFICATION_ID, buildNotification())
    }

    private fun buildNotification(): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val appIntent = PendingIntent.getActivity(
            this,
            0,
            launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val lockIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, DesktopLyricService::class.java).setAction(ACTION_TOGGLE_LOCK),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val closeIntent = PendingIntent.getService(
            this,
            2,
            Intent(this, DesktopLyricService::class.java).setAction(ACTION_CLOSE),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val lockLabel = if (locked) "解锁" else "锁定"
        // A locked overlay is click-through, so tapping the notification body is
        // the most discoverable way to unlock; otherwise it just opens the app.
        val contentIntent = if (locked) lockIntent else appIntent
        val contentText = if (locked) "已锁定，点击解锁" else "正在显示悬浮歌词"
        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("桌面歌词")
            .setContentText(contentText)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setOngoing(true)
            .setContentIntent(contentIntent)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(0, lockLabel, lockIntent)
            .addAction(0, "关闭", closeIntent)
        return builder.build()
    }

    companion object {
        private const val TAG = "WelkinLyric"
        private const val CHANNEL_ID = "desktop-lyric"
        private const val NOTIFICATION_ID = 0x4C59
        private const val PREFS = "desktop-lyrics"
        private const val POSITION_X = "position-x"
        private const val POSITION_Y = "position-y"
        const val ACTION_TOGGLE_LOCK = "com.atemukesu.welkinplayer.DESKTOP_LYRIC_TOGGLE_LOCK"
        const val ACTION_CLOSE = "com.atemukesu.welkinplayer.DESKTOP_LYRIC_CLOSE"
    }
}
