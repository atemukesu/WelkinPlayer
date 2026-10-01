package com.atemukesu.welkinplayer

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import android.util.Base64
import org.json.JSONObject

/**
 * Foreground service that owns a native [MediaSession] and its media-style
 * notification.
 *
 * The WebView keeps doing the actual audio decoding; this service only mirrors
 * its state onto the lock screen / notification shade / Bluetooth controls and
 * forwards transport buttons back to the main window through
 * [MediaControlBridge.control]. The platform [MediaSession] is used directly so
 * no extra AndroidX Media dependency is required.
 */
class MediaPlaybackService : Service() {
    private var session: MediaSession? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        session = MediaSession(this, "Welkin").apply {
            setFlags(
                MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or
                    MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS
            )
            setCallback(object : MediaSession.Callback() {
                override fun onPlay() = MediaControlBridge.control(actionJson("play"))
                override fun onPause() = MediaControlBridge.control(actionJson("pause"))
                override fun onStop() = MediaControlBridge.control(actionJson("stop"))
                override fun onSkipToNext() = MediaControlBridge.control(actionJson("next"))
                override fun onSkipToPrevious() =
                    MediaControlBridge.control(actionJson("previous"))

                override fun onSeekTo(pos: Long) =
                    MediaControlBridge.control(actionJson("seek", pos.toDouble()))
            })
            isActive = true
        }
        startForegroundCompat(buildNotification("Welkin", "", false, null))
        MediaControlBridge.attach(this)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_TOGGLE -> MediaControlBridge.control(actionJson("toggle"))
            ACTION_NEXT -> MediaControlBridge.control(actionJson("next"))
            ACTION_PREVIOUS -> MediaControlBridge.control(actionJson("previous"))
        }
        return START_NOT_STICKY
    }

    /** The app's task was swiped away: the notification should go with it. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        session?.isActive = false
        session?.release()
        session = null
        MediaControlBridge.detach()
        super.onDestroy()
    }

    /** Apply a metadata / playback snapshot pushed from Rust. */
    fun update(json: String) {
        val payload = try {
            JSONObject(json)
        } catch (error: Throwable) {
            return
        }
        val title = payload.optString("title")
        val artist = payload.optString("artist")
        val album = payload.optString("album")
        val playing = payload.optBoolean("playing")
        val position = payload.optDouble("positionMs", 0.0).toLong()
        val duration = payload.optDouble("durationMs", 0.0).toLong()
        val art = decodeCover(payload.optString("coverBase64", null))

        val metadata = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
            .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
            .putLong(MediaMetadata.METADATA_KEY_DURATION, duration)
        if (art != null) metadata.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art)
        session?.setMetadata(metadata.build())

        val state = if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED
        val actions = (
            PlaybackState.ACTION_PLAY or
                PlaybackState.ACTION_PAUSE or
                PlaybackState.ACTION_PLAY_PAUSE or
                PlaybackState.ACTION_STOP or
                PlaybackState.ACTION_SKIP_TO_NEXT or
                PlaybackState.ACTION_SKIP_TO_PREVIOUS or
                PlaybackState.ACTION_SEEK_TO
            ).toLong()
        val playback = PlaybackState.Builder()
            .setActions(actions)
            .setState(state.toLong(), position, if (playing) 1f else 0f)
            .build()
        session?.setPlaybackState(playback)

        val notification = buildNotification(title, artist, playing, art)
        if (playing) {
            startForegroundCompat(notification)
        } else {
            // Keep the notification visible while paused, without forcing the
            // service into the foreground state until playback resumes.
            getSystemService(NotificationManager::class.java)?.notify(NOTIFICATION_ID, notification)
            stopForeground(false)
        }
    }

    private fun decodeCover(encoded: String?): Bitmap? {
        if (encoded.isNullOrEmpty()) return null
        return try {
            val bytes = Base64.decode(encoded, Base64.DEFAULT)
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
        } catch (error: Throwable) {
            null
        }
    }

    private fun actionJson(action: String, value: Double? = null): String {
        val json = JSONObject()
        json.put("action", action)
        if (value != null) json.put("value", value)
        return json.toString()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            "Media playback",
            NotificationManager.IMPORTANCE_LOW
        )
        channel.setShowBadge(false)
        manager.createNotificationChannel(channel)
    }

    private fun startForegroundCompat(notification: Notification) {
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun buildNotification(
        title: String,
        artist: String,
        playing: Boolean,
        art: Bitmap?
    ): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val contentIntent = PendingIntent.getActivity(
            this,
            0,
            launch,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val previousIntent = PendingIntent.getService(
            this,
            11,
            Intent(this, MediaPlaybackService::class.java).setAction(ACTION_PREVIOUS),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val toggleIntent = PendingIntent.getService(
            this,
            12,
            Intent(this, MediaPlaybackService::class.java).setAction(ACTION_TOGGLE),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val nextIntent = PendingIntent.getService(
            this,
            13,
            Intent(this, MediaPlaybackService::class.java).setAction(ACTION_NEXT),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val style = Notification.MediaStyle()
            .setMediaSession(session?.sessionToken)
            .setShowActionsInCompactView(0, 1, 2)

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        builder
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(title)
            .setContentText(artist)
            .setContentIntent(contentIntent)
            .setStyle(style)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setOngoing(playing)
            .addAction(Notification.Action.Builder(null, "Previous", previousIntent).build())
            .addAction(
                Notification.Action.Builder(
                    null,
                    if (playing) "Pause" else "Play",
                    toggleIntent
                ).build()
            )
            .addAction(Notification.Action.Builder(null, "Next", nextIntent).build())
        if (art != null) builder.setLargeIcon(art)
        return builder.build()
    }

    companion object {
        private const val CHANNEL_ID = "media-playback"
        private const val NOTIFICATION_ID = 0x4D45
        const val ACTION_TOGGLE = "com.atemukesu.welkinplayer.MEDIA_TOGGLE"
        const val ACTION_NEXT = "com.atemukesu.welkinplayer.MEDIA_NEXT"
        const val ACTION_PREVIOUS = "com.atemukesu.welkinplayer.MEDIA_PREVIOUS"
    }
}
