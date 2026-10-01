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
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.webkit.JavascriptInterface
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.core.app.NotificationCompat

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
    private var touchStartX = 0
    private var touchStartY = 0
    private var touchRawX = 0f
    private var touchRawY = 0f

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        startForegroundCompat(buildNotification())
        createOverlay()
        // Only now are we able to accept the documents that were queued while
        // the service was starting.
        DesktopLyricBridge.attachService(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_TOGGLE_LOCK -> {
                setLocked(!locked)
                return START_STICKY
            }
            ACTION_CLOSE -> {
                stopSelf()
                return START_NOT_STICKY
            }
        }
        // A plain start (no action): load the renderer once.
        loadRenderer()
        return START_STICKY
    }

    override fun onDestroy() {
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
        main.post {
            webView?.visibility = if (value) View.VISIBLE else View.GONE
        }
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
                if (combined.isNotBlank()) view?.evaluateJavascript(combined, null)
            }
        }
        view.addJavascriptInterface(BridgeInterface(), "AndroidDesktopLyric")
        view.setOnTouchListener { _, event ->
            val layout = params ?: return@setOnTouchListener false
            when (event.action) {
                MotionEvent.ACTION_DOWN -> {
                    touchStartX = layout.x
                    touchStartY = layout.y
                    touchRawX = event.rawX
                    touchRawY = event.rawY
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    layout.x = touchStartX + (event.rawX - touchRawX).toInt()
                    layout.y = touchStartY + (event.rawY - touchRawY).toInt()
                    try {
                        windowManager?.updateViewLayout(view, layout)
                    } catch (_: Throwable) {
                    }
                    true
                }
                else -> false
            }
        }

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
            x = 0
            y = (resources.displayMetrics.heightPixels * 0.55f).toInt()
        }

        view.visibility = View.GONE
        try {
            manager.addView(view, layout)
        } catch (error: Throwable) {
            // Permission revoked between the check and now, or an incompatible
            // OEM overlay policy: bail out instead of crashing the process.
            view.destroy()
            stopSelf()
            return
        }
        webView = view
        params = layout
    }

    private fun loadRenderer() {
        val view = webView ?: return
        val html = DesktopLyricBridge.html
        if (html.isBlank()) return
        pageReady = false
        view.loadDataWithBaseURL("https://desktop-lyric.local/", html, "text/html", "utf-8", null)
    }

    /** JS surface the renderer uses to drive the native window. */
    private inner class BridgeInterface {
        @JavascriptInterface
        fun setLocked(value: Boolean) = this@DesktopLyricService.setLocked(value)

        @JavascriptInterface
        fun setVisible(value: Boolean) = this@DesktopLyricService.setVisible(value)
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
        val contentIntent = PendingIntent.getActivity(
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
        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("桌面歌词")
            .setContentText("正在显示悬浮歌词")
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setOngoing(true)
            .setContentIntent(contentIntent)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(0, lockLabel, lockIntent)
            .addAction(0, "关闭", closeIntent)
        return builder.build()
    }

    companion object {
        private const val CHANNEL_ID = "desktop-lyric"
        private const val NOTIFICATION_ID = 0x4C59
        const val ACTION_TOGGLE_LOCK = "com.atemukesu.welkinplayer.DESKTOP_LYRIC_TOGGLE_LOCK"
        const val ACTION_CLOSE = "com.atemukesu.welkinplayer.DESKTOP_LYRIC_CLOSE"
    }
}
