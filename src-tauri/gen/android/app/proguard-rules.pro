# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# Desktop lyric bridge: reached only from Rust over JNI (by class and method
# name), so R8 cannot see any references and would strip the whole class in a
# minified release build, producing a ClassNotFoundException at runtime.
-keep class com.atemukesu.welkinplayer.DesktopLyricBridge { *; }

# Media control bridge + service: the bridge is reached only from Rust over JNI
# (by class and method name), so R8 cannot see any references and would strip it
# in a minified release build.
-keep class com.atemukesu.welkinplayer.MediaControlBridge { *; }
-keep class com.atemukesu.welkinplayer.MediaPlaybackService { *; }