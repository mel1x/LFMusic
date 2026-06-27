package lfmusic.app

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Base64
import android.view.View
import android.view.WindowManager
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import org.json.JSONObject

private const val AUDIO_PERMISSION_REQUEST = 42
private const val MEDIA_NOTIFICATION_ID = 1001
private const val AUDIO_FEATURE_NOTIFICATION_ID = 1002
private const val MEDIA_CHANNEL_ID = "lfmusic_playback"
private const val AUDIO_FEATURE_CHANNEL_ID = "lfmusic_audio_analysis"
private const val ACTION_MEDIA_COMMAND = "lfmusic.app.MEDIA_COMMAND"
private const val EXTRA_MEDIA_COMMAND = "media_command"
private const val DEFAULT_SYSTEM_BAR_COLOR = 0xFF101010.toInt()

class MainActivity : TauriActivity() {
  private var appWebView: WebView? = null
  private lateinit var mediaSession: MediaSession
  private lateinit var notificationManager: NotificationManager

  private var mediaHasTrack = false
  private var mediaPlaying = false
  private var mediaTitle = "LFMusic"
  private var mediaArtist = ""
  private var mediaAlbum = ""
  private var mediaDurationMs = 0L
  private var mediaPositionMs = 0L
  private var mediaArtwork: Bitmap? = null
  private var mediaArtworkSource = ""
  private var systemBarColor = DEFAULT_SYSTEM_BAR_COLOR
  private val notificationHandler = Handler(Looper.getMainLooper())

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    setupSystemBars()
    setupMediaSession()
    requestRequiredPermissions()
    handleMediaIntent(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    handleMediaIntent(intent)
  }

  override fun onResume() {
    super.onResume()
    appWebView?.let { publishPermissionStatus(it) }
  }

  override fun onRequestPermissionsResult(
    requestCode: Int,
    permissions: Array<out String>,
    grantResults: IntArray,
  ) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    if (requestCode == AUDIO_PERMISSION_REQUEST) {
      appWebView?.let { publishPermissionStatus(it) }
    }
  }

  @SuppressLint("AddJavascriptInterface")
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    appWebView = webView
    setupEdgeToEdgeWebView(webView)
    webView.addJavascriptInterface(AndroidMediaBridge(), "LFMusicAndroid")
    installInsetsBridge(webView)
    publishPermissionStatus(webView)
  }

  override fun onDestroy() {
    if (::notificationManager.isInitialized) {
      notificationManager.cancel(MEDIA_NOTIFICATION_ID)
      notificationManager.cancel(AUDIO_FEATURE_NOTIFICATION_ID)
    }
    if (::mediaSession.isInitialized) {
      mediaSession.release()
    }
    notificationHandler.removeCallbacksAndMessages(null)
    super.onDestroy()
  }

  private fun requestRequiredPermissions() {
    val permissions = mutableListOf<String>()
    val audioPermission = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      Manifest.permission.READ_MEDIA_AUDIO
    } else {
      Manifest.permission.READ_EXTERNAL_STORAGE
    }

    if (ContextCompat.checkSelfPermission(this, audioPermission) != PackageManager.PERMISSION_GRANTED) {
      permissions.add(audioPermission)
    }

    if (
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
      ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) {
      permissions.add(Manifest.permission.POST_NOTIFICATIONS)
    }

    if (permissions.isNotEmpty()) {
      ActivityCompat.requestPermissions(this, permissions.toTypedArray(), AUDIO_PERMISSION_REQUEST)
    } else {
      appWebView?.let { publishPermissionStatus(it) }
    }
  }

  private fun mediaPermissionName(): String {
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      Manifest.permission.READ_MEDIA_AUDIO
    } else {
      Manifest.permission.READ_EXTERNAL_STORAGE
    }
  }

  private fun permissionStatusJson(): String {
    val mediaPermission = mediaPermissionName()
    val mediaGranted = ContextCompat.checkSelfPermission(this, mediaPermission) == PackageManager.PERMISSION_GRANTED
    val notificationsRequired = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
    val notificationsGranted = !notificationsRequired ||
      ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    return JSONObject()
      .put("mediaGranted", mediaGranted)
      .put("notificationsGranted", notificationsGranted)
      .put("notificationsRequired", notificationsRequired)
      .toString()
  }

  private fun publishPermissionStatus(webView: WebView) {
    val payload = JSONObject.quote(permissionStatusJson())
    val js = """
      window.dispatchEvent(new CustomEvent('lfmusic:android-permissions', {
        detail: JSON.parse($payload)
      }));
    """.trimIndent()
    webView.post { webView.evaluateJavascript(js, null) }
  }

  private fun setupSystemBars() {
    WindowCompat.setDecorFitsSystemWindows(window, false)
    window.decorView.systemUiVisibility =
      View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
      View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or
      View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION

    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      window.attributes = window.attributes.apply {
        layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
      }
    }

    applySystemBarsColor(systemBarColor)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      window.isStatusBarContrastEnforced = false
      window.isNavigationBarContrastEnforced = false
    }

    val controller = WindowInsetsControllerCompat(window, window.decorView)
    controller.isAppearanceLightStatusBars = false
    controller.isAppearanceLightNavigationBars = false
  }

  private fun setupEdgeToEdgeWebView(webView: WebView) {
    WindowCompat.setDecorFitsSystemWindows(window, false)
    webView.fitsSystemWindows = false
    webView.setPadding(0, 0, 0, 0)
    webView.setBackgroundColor(Color.TRANSPARENT)
  }

  private fun applySystemBarsColor(color: Int) {
    systemBarColor = color
    window.statusBarColor = Color.TRANSPARENT
    window.navigationBarColor = color

    val controller = WindowInsetsControllerCompat(window, window.decorView)
    controller.isAppearanceLightStatusBars = false
    controller.isAppearanceLightNavigationBars = false
  }

  private fun setupMediaSession() {
    notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val channel = NotificationChannel(
        MEDIA_CHANNEL_ID,
        "LFMusic playback",
        NotificationManager.IMPORTANCE_LOW,
      ).apply {
        description = "LFMusic media controls"
        setShowBadge(false)
      }
      notificationManager.createNotificationChannel(channel)

      val analysisChannel = NotificationChannel(
        AUDIO_FEATURE_CHANNEL_ID,
        "LFMusic audio analysis",
        NotificationManager.IMPORTANCE_LOW,
      ).apply {
        description = "Background audio analysis progress"
        setShowBadge(false)
      }
      notificationManager.createNotificationChannel(analysisChannel)
    }

    mediaSession = MediaSession(this, "LFMusic").apply {
      setCallback(object : MediaSession.Callback() {
        override fun onPlay() = sendPlayerCommand("play")
        override fun onPause() = sendPlayerCommand("pause")
        override fun onSkipToNext() = sendPlayerCommand("next")
        override fun onSkipToPrevious() = sendPlayerCommand("previous")
      })
      isActive = false
    }
  }

  private fun installInsetsBridge(webView: WebView) {
    val publishInsets = { windowInsets: WindowInsetsCompat ->
      val mask = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      publishSystemInsets(webView, withResourceFallbacks(windowInsets.getInsets(mask)))
      windowInsets
    }
    ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { _, windowInsets ->
      publishInsets(windowInsets)
    }
    ViewCompat.setOnApplyWindowInsetsListener(webView) { view, windowInsets ->
      view.setPadding(0, 0, 0, 0)
      publishInsets(windowInsets)
    }
    ViewCompat.requestApplyInsets(window.decorView)
    ViewCompat.requestApplyInsets(webView)

    webView.post {
      ViewCompat.requestApplyInsets(window.decorView)
      ViewCompat.requestApplyInsets(webView)
      publishSystemInsets(webView, fallbackSystemInsets())
    }
  }

  private fun publishSystemInsets(webView: WebView, insets: Insets) {
    val payload = systemInsetsJson(insets)
    val js = """
      (() => {
        const detail = $payload;
        const root = document.documentElement;
        if (!root) return;
        root.style.setProperty('--android-safe-top', detail.top + 'px');
        root.style.setProperty('--android-safe-right', detail.right + 'px');
        root.style.setProperty('--android-safe-bottom', detail.bottom + 'px');
        root.style.setProperty('--android-safe-left', detail.left + 'px');
        window.dispatchEvent(new CustomEvent('lfmusic:android-insets', { detail }));
      })();
    """.trimIndent()
    webView.post { webView.evaluateJavascript(js, null) }
  }

  private fun currentSystemInsets(): Insets {
    val rootInsets = ViewCompat.getRootWindowInsets(window.decorView)
    val mask = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
    return withResourceFallbacks(rootInsets?.getInsets(mask) ?: fallbackSystemInsets())
  }

  private fun systemInsetsJson(insets: Insets): String {
    val density = resources.displayMetrics.density.coerceAtLeast(1f)
    val top = insets.top / density
    val right = insets.right / density
    val bottom = insets.bottom / density
    val left = insets.left / density
    return JSONObject()
      .put("top", top)
      .put("right", right)
      .put("bottom", bottom)
      .put("left", left)
      .toString()
  }

  private fun withResourceFallbacks(insets: Insets): Insets {
    return Insets.of(
      insets.left,
      if (insets.top > 0) insets.top else androidDimensionPx("status_bar_height"),
      insets.right,
      if (insets.bottom > 0) insets.bottom else androidDimensionPx("navigation_bar_height"),
    )
  }

  private fun fallbackSystemInsets(): Insets {
    return Insets.of(
      0,
      androidDimensionPx("status_bar_height"),
      0,
      androidDimensionPx("navigation_bar_height"),
    )
  }

  private fun androidDimensionPx(name: String): Int {
    val id = resources.getIdentifier(name, "dimen", "android")
    return if (id > 0) resources.getDimensionPixelSize(id) else 0
  }

  private fun updateMediaSessionFromJson(payload: String) {
    if (!::mediaSession.isInitialized) setupMediaSession()

    val json = JSONObject(payload)
    mediaHasTrack = json.optBoolean("hasTrack", false)
    if (!mediaHasTrack) {
      mediaSession.isActive = false
      notificationManager.cancel(MEDIA_NOTIFICATION_ID)
      mediaArtwork = null
      mediaArtworkSource = ""
      return
    }

    mediaTitle = json.optString("title", "LFMusic").ifBlank { "LFMusic" }
    mediaArtist = json.optString("artist", "")
    mediaAlbum = json.optString("album", "")
    mediaPlaying = json.optString("state", "Stopped") == "Playing"
    mediaDurationMs = (json.optDouble("durationSecs", 0.0) * 1000.0).toLong().coerceAtLeast(0L)
    mediaPositionMs = (json.optDouble("positionSecs", 0.0) * 1000.0).toLong().coerceAtLeast(0L)
    if (json.has("artwork")) {
      updateArtwork(json.optString("artwork", ""))
    }

    updateNativeMediaSession()
    updateMediaNotification()
  }

  private fun updateNativeMediaSession() {
    val metadataBuilder = MediaMetadata.Builder()
      .putString(MediaMetadata.METADATA_KEY_TITLE, mediaTitle)
      .putString(MediaMetadata.METADATA_KEY_ARTIST, mediaArtist)
      .putString(MediaMetadata.METADATA_KEY_ALBUM, mediaAlbum)
      .putLong(MediaMetadata.METADATA_KEY_DURATION, mediaDurationMs)

    mediaArtwork?.let { artwork ->
      metadataBuilder
        .putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, artwork)
        .putBitmap(MediaMetadata.METADATA_KEY_ART, artwork)
    }

    val metadata = metadataBuilder.build()

    val state = if (mediaPlaying) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED
    val actions = PlaybackState.ACTION_PLAY or
      PlaybackState.ACTION_PAUSE or
      PlaybackState.ACTION_PLAY_PAUSE or
      PlaybackState.ACTION_SKIP_TO_NEXT or
      PlaybackState.ACTION_SKIP_TO_PREVIOUS

    val playbackState = PlaybackState.Builder()
      .setActions(actions)
      .setState(state, mediaPositionMs, if (mediaPlaying) 1f else 0f)
      .build()

    mediaSession.setMetadata(metadata)
    mediaSession.setPlaybackState(playbackState)
    mediaSession.isActive = true
  }

  private fun updateMediaNotification() {
    if (!mediaHasTrack) {
      notificationManager.cancel(MEDIA_NOTIFICATION_ID)
      return
    }

    if (
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
      ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) {
      return
    }

    val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      Notification.Builder(this, MEDIA_CHANNEL_ID)
    } else {
      @Suppress("DEPRECATION")
      Notification.Builder(this)
    }

    val text = when {
      mediaArtist.isNotBlank() -> mediaArtist
      mediaAlbum.isNotBlank() -> mediaAlbum
      else -> "LFMusic"
    }

    val playPauseCommand = if (mediaPlaying) "pause" else "play"
    val playPauseIcon = if (mediaPlaying) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play
    val playPauseTitle = if (mediaPlaying) "Pause" else "Play"
    mediaArtwork?.let { builder.setLargeIcon(it) }

    val notification = builder
      .setSmallIcon(R.drawable.ic_notification)
      .setContentTitle(mediaTitle)
      .setContentText(text)
      .setSubText(mediaAlbum.takeIf { it.isNotBlank() })
      .setContentIntent(mediaPendingIntent("open"))
      .setShowWhen(false)
      .setOnlyAlertOnce(true)
      .setOngoing(mediaPlaying)
      .setCategory(Notification.CATEGORY_TRANSPORT)
      .setVisibility(Notification.VISIBILITY_PUBLIC)
      .addAction(buildNotificationAction("previous", android.R.drawable.ic_media_previous, "Previous"))
      .addAction(buildNotificationAction(playPauseCommand, playPauseIcon, playPauseTitle))
      .addAction(buildNotificationAction("next", android.R.drawable.ic_media_next, "Next"))
      .setStyle(
        Notification.MediaStyle()
          .setMediaSession(mediaSession.sessionToken)
          .setShowActionsInCompactView(0, 1, 2)
      )
      .build()

    notificationManager.notify(MEDIA_NOTIFICATION_ID, notification)
  }

  private fun updateAudioFeatureNotificationFromJson(payload: String) {
    if (!::notificationManager.isInitialized) {
      notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    }

    if (
      Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
      ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) {
      return
    }

    val json = JSONObject(payload)
    val total = json.optInt("total", 0).coerceAtLeast(0)
    if (total <= 0) {
      notificationManager.cancel(AUDIO_FEATURE_NOTIFICATION_ID)
      return
    }

    val current = json.optInt("current", 0).coerceIn(0, total)
    val active = json.optBoolean("active", current < total) && current < total
    val title = json.optString("title", "Background recommendation prep")
      .ifBlank { "Background recommendation prep" }
    val text = json.optString("text", "Processed $current of $total tracks")
      .ifBlank { "Processed $current of $total tracks" }

    val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      Notification.Builder(this, AUDIO_FEATURE_CHANNEL_ID)
    } else {
      @Suppress("DEPRECATION")
      Notification.Builder(this)
    }

    @Suppress("DEPRECATION")
    val notification = builder
      .setSmallIcon(R.drawable.ic_notification)
      .setContentTitle(title)
      .setContentText(text)
      .setSubText("$current / $total")
      .setContentIntent(mediaPendingIntent("open"))
      .setShowWhen(false)
      .setOnlyAlertOnce(true)
      .setOngoing(active)
      .setAutoCancel(!active)
      .setCategory(Notification.CATEGORY_PROGRESS)
      .setVisibility(Notification.VISIBILITY_PUBLIC)
      .setLocalOnly(true)
      .setProgress(total, current, false)
      .setPriority(Notification.PRIORITY_LOW)
      .build()

    notificationHandler.removeCallbacksAndMessages(AUDIO_FEATURE_NOTIFICATION_ID)
    notificationManager.notify(AUDIO_FEATURE_NOTIFICATION_ID, notification)

    if (!active) {
      notificationHandler.postAtTime(
        { notificationManager.cancel(AUDIO_FEATURE_NOTIFICATION_ID) },
        AUDIO_FEATURE_NOTIFICATION_ID,
        android.os.SystemClock.uptimeMillis() + 2200L,
      )
    }
  }

  private fun buildNotificationAction(command: String, icon: Int, title: String): Notification.Action {
    return Notification.Action.Builder(icon, title, mediaPendingIntent(command)).build()
  }

  private fun updateArtwork(source: String) {
    if (source == mediaArtworkSource) return
    mediaArtworkSource = source
    mediaArtwork = decodeArtwork(source)
  }

  private fun decodeArtwork(source: String): Bitmap? {
    if (source.isBlank()) return null

    val marker = "base64,"
    val base64Start = source.indexOf(marker)
    if (base64Start < 0) return null

    return try {
      val encoded = source.substring(base64Start + marker.length)
      val bytes = Base64.decode(encoded, Base64.DEFAULT)
      val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return null
      val maxSide = maxOf(bitmap.width, bitmap.height)
      if (maxSide <= 512) {
        bitmap
      } else {
        val scale = 512f / maxSide.toFloat()
        Bitmap.createScaledBitmap(
          bitmap,
          (bitmap.width * scale).toInt().coerceAtLeast(1),
          (bitmap.height * scale).toInt().coerceAtLeast(1),
          true,
        )
      }
    } catch (_: Exception) {
      null
    }
  }

  private fun mediaPendingIntent(command: String): PendingIntent {
    val intent = Intent(this, MainActivity::class.java).apply {
      action = ACTION_MEDIA_COMMAND
      putExtra(EXTRA_MEDIA_COMMAND, command)
      flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    return PendingIntent.getActivity(this, command.hashCode(), intent, flags)
  }

  private fun handleMediaIntent(intent: Intent?) {
    if (intent?.action != ACTION_MEDIA_COMMAND) return
    val command = intent.getStringExtra(EXTRA_MEDIA_COMMAND) ?: return
    if (command != "open") sendPlayerCommand(command)
  }

  private fun sendPlayerCommand(command: String) {
    val webView = appWebView ?: return
    val quotedCommand = JSONObject.quote(command)
    val js = """
      window.dispatchEvent(new CustomEvent('lfmusic:android-media-command', {
        detail: { command: $quotedCommand }
      }));
    """.trimIndent()
    webView.post { webView.evaluateJavascript(js, null) }
  }

  inner class AndroidMediaBridge {
    @JavascriptInterface
    fun getPermissionStatus(): String {
      return permissionStatusJson()
    }

    @JavascriptInterface
    fun getSystemInsets(): String {
      return systemInsetsJson(currentSystemInsets())
    }

    @JavascriptInterface
    fun requestRequiredPermissionsFromJs() {
      runOnUiThread {
        requestRequiredPermissions()
      }
    }

    @JavascriptInterface
    fun updateMediaSession(payload: String) {
      runOnUiThread {
        try {
          updateMediaSessionFromJson(payload)
        } catch (_: Exception) {
          if (::mediaSession.isInitialized) {
            mediaSession.isActive = false
          }
          if (::notificationManager.isInitialized) {
            notificationManager.cancel(MEDIA_NOTIFICATION_ID)
          }
        }
      }
    }

    @JavascriptInterface
    fun updateSystemBars(color: String) {
      runOnUiThread {
        try {
          applySystemBarsColor(Color.parseColor(color))
        } catch (_: Exception) {
          applySystemBarsColor(DEFAULT_SYSTEM_BAR_COLOR)
        }
      }
    }

    @JavascriptInterface
    fun updateAudioFeatureProgress(payload: String) {
      runOnUiThread {
        try {
          updateAudioFeatureNotificationFromJson(payload)
        } catch (_: Exception) {
          if (::notificationManager.isInitialized) {
            notificationManager.cancel(AUDIO_FEATURE_NOTIFICATION_ID)
          }
        }
      }
    }
  }
}
