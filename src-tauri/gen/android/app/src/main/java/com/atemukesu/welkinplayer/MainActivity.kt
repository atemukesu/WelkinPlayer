package com.atemukesu.welkinplayer

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  override fun onDestroy() {
    // Quitting the app must also tear the floating lyrics down; backgrounding
    // it (home button) keeps them, which is the point of an overlay.
    if (isFinishing) DesktopLyricBridge.stop(this)
    super.onDestroy()
  }
}
