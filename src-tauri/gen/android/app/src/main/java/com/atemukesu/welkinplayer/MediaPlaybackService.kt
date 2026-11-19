/*
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

package com.atemukesu.welkinplayer

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import android.util.Base64
import androidx.core.content.ContextCompat
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
    private var audioManager: AudioManager? = null
    private var focusRequest: AudioFocusRequest? = null
    /** Whether the OS currently grants us audio focus. */
    private var focusHeld = false
    /** The last play/pause state pushed from Rust. */
    private var lastPlaying = false
    /** True when focus loss paused us and focus gain should resume. */
    private var resumeAfterFocusGain = false
    private var noisyReceiverRegistered = false

    /**
     * Android broadcasts this just before audio is rerouted to the built-in
     * speaker (wired headset unplugged, A2DP sink disconnected). Pause so the
     * music does not suddenly blast out of the phone speaker.
     */
    private val noisyReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action != AudioManager.ACTION_AUDIO_BECOMING_NOISY) return
            // Do not auto-resume after a headphone disconnect: the user chose
            // to remove the output, not to pause temporarily.
            resumeAfterFocusGain = false
            if (lastPlaying) MediaControlBridge.control(actionJson("pause"))
        }
    }

    private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
        when (change) {
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT,
            AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                // A phone call (or another app) took focus. Pause now and ask
                // Rust to rebuild the output stream when focus returns: the OS
                // commonly disconnects/suspends AAudio during a call even
                // though the connected-device list never changes.
                if (lastPlaying) {
                    resumeAfterFocusGain = true
                    MediaControlBridge.control(actionJson("pause"))
                }
            }
            AudioManager.AUDIOFOCUS_GAIN -> {
                if (resumeAfterFocusGain) {
                    resumeAfterFocusGain = false
                    MediaControlBridge.control(actionJson("resume"))
                }
                focusHeld = true
            }
            AudioManager.AUDIOFOCUS_LOSS -> {
                resumeAfterFocusGain = false
                focusHeld = false
                if (lastPlaying) MediaControlBridge.control(actionJson("pause"))
            }
        }
    }

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
        requestAudioFocus()
        registerNoisyReceiver()
        try {
            startForegroundCompat(buildNotification("Welkin", "", false, null))
        } catch (error: Throwable) {
            // e.g. ForegroundServiceStartNotAllowedException / SecurityException.
            // Never let this crash the process - that would also take the lyric
            // overlay down. Abandon the service instead.
            session?.release()
            session = null
            unregisterNoisyReceiver()
            abandonAudioFocus()
            MediaControlBridge.detach()
            stopSelf()
            return
        }
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
        unregisterNoisyReceiver()
        abandonAudioFocus()
        session?.isActive = false
        session?.release()
        session = null
        MediaControlBridge.detach()
        super.onDestroy()
    }

    /** Apply a metadata / playback snapshot pushed from Rust. */
    fun update(json: String) {
        try {
            applySnapshot(json)
        } catch (error: Throwable) {
            // A Java exception must never cross the JNI boundary: it would stay
            // pending on the Android main thread and break every later JNI call.
        }
    }

    /**
     * Correct only the playhead of the running session, without touching the
     * metadata or the notification. The platform extrapolates position from the
     * last `setPlaybackState`, so this is how a loop/seek (or the periodic
     * position tick) resets the reported time.
     */
    fun updatePosition(positionMs: Long, playing: Boolean) {
        try {
            session?.setPlaybackState(buildPlaybackState(playing, positionMs))
        } catch (error: Throwable) {
            // Never let a Java exception cross the JNI boundary.
        }
    }

    private fun buildPlaybackState(playing: Boolean, position: Long): PlaybackState {
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
        return PlaybackState.Builder()
            .setActions(actions)
            .setState(state, position, if (playing) 1f else 0f)
            .build()
    }

    private fun applySnapshot(json: String) {
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
        val wasPlaying = lastPlaying
        lastPlaying = playing
        // A foreground service started while another app held focus may have
        // been denied; retry on an actual pause -> play transition only, so a
        // permanent focus loss does not turn into a focus tug-of-war.
        if (playing && !wasPlaying && !focusHeld) requestAudioFocus()

        val metadata = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
            .putString(MediaMetadata.METADATA_KEY_ALBUM, album)
            .putLong(MediaMetadata.METADATA_KEY_DURATION, duration)
        if (art != null) metadata.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art)
        session?.setMetadata(metadata.build())
        session?.setPlaybackState(buildPlaybackState(playing, position))

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

    /**
     * Listen for the system hint that audio is about to be rerouted to the
     * built-in speaker. Registered for the service lifetime (not just while
     * playing) so a disconnect arriving between two snapshots is still caught;
     * the handler itself only pauses when the last snapshot said "playing".
     */
    private fun registerNoisyReceiver() {
        if (noisyReceiverRegistered) return
        try {
            ContextCompat.registerReceiver(
                this,
                noisyReceiver,
                IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
                ContextCompat.RECEIVER_NOT_EXPORTED,
            )
            noisyReceiverRegistered = true
        } catch (error: Throwable) {
            // Registration must never take the service down.
            noisyReceiverRegistered = false
        }
    }

    private fun unregisterNoisyReceiver() {
        if (!noisyReceiverRegistered) return
        noisyReceiverRegistered = false
        try {
            unregisterReceiver(noisyReceiver)
        } catch (error: Throwable) {
            // Already unregistered (e.g. service teardown race).
        }
    }

    /**
     * Request media focus so a phone call or another app can tell us to pause
     * and resume. Without this the OS may silently suspend the AAudio stream
     * while leaving our transport marked "playing".
     */
    private fun requestAudioFocus() {
        val manager = getSystemService(AudioManager::class.java) ?: return
        audioManager = manager
        val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(attributes)
            .setAcceptsDelayedFocusGain(false)
            .setOnAudioFocusChangeListener(focusListener)
            .build()
        focusRequest = request
        focusHeld = manager.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
    }

    private fun abandonAudioFocus() {
        focusRequest?.let { request ->
            audioManager?.abandonAudioFocusRequest(request)
        }
        focusRequest = null
        audioManager = null
        focusHeld = false
        resumeAfterFocusGain = false
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
