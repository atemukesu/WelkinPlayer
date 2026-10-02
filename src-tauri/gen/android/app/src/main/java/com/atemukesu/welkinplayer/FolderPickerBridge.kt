package com.atemukesu.welkinplayer

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import android.provider.Settings
import android.util.Log
import androidx.activity.result.ActivityResultLauncher
import androidx.core.content.ContextCompat

/**
 * System folder picker for local music sources.
 *
 * Tauri's dialog plugin cannot pick folders on Android
 * (`FolderPickerNotImplemented`), so the main window drives the Storage Access
 * Framework directly instead: Rust starts a pick, the result is parked here and
 * polled back over JNI.
 *
 * The chosen tree URI is mapped back to a `/storage/...` path so the existing
 * `std::fs`-based local backend keeps working unchanged. On Android 11+,
 * however, scoped storage blocks directory enumeration (`readdir`) in shared
 * storage even with the media read permission, so folder listing requires "All
 * files access" (`MANAGE_EXTERNAL_STORAGE`). When it is missing, [pick] opens
 * the system settings screen and reports [PICK_NEEDS_ALL_FILES] so the UI can
 * ask the user to retry.
 */
object FolderPickerBridge {
    private const val TAG = "WelkinFolder"

    /** `pick()` outcome reported back to Rust. */
    const val PICK_LAUNCHED = 0
    const val PICK_NEEDS_ALL_FILES = 1
    const val PICK_UNAVAILABLE = -1

    private val main = Handler(Looper.getMainLooper())
    private val lock = Any()

    private var activity: Activity? = null
    private var folderLauncher: ActivityResultLauncher<Intent>? = null
    private var permissionLauncher: ActivityResultLauncher<String>? = null

    private var pickActive = false
    private var resultReady = false
    private var resultValue: String? = null
    private var pendingPath: String? = null

    fun attach(value: Activity) {
        activity = value
    }

    fun detach(value: Activity) {
        if (activity === value) activity = null
    }

    fun registerFolderLauncher(launcher: ActivityResultLauncher<Intent>) {
        folderLauncher = launcher
    }

    fun registerPermissionLauncher(launcher: ActivityResultLauncher<String>) {
        permissionLauncher = launcher
    }

    /** Called from Rust/the UI: whether local folders can be read right now. */
    @JvmStatic
    fun hasAccess(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.R || Environment.isExternalStorageManager()

    /** Called from Rust/the UI: open the "all files access" settings screen. */
    @JvmStatic
    fun requestAccess() {
        val act = activity ?: return
        main.post { openAllFilesSettings(act) }
    }

    /** Called from Rust. Returns one of the `PICK_*` constants. */
    @JvmStatic
    fun pick(): Int {
        val launcher = folderLauncher ?: return PICK_UNAVAILABLE
        val act = activity ?: return PICK_UNAVAILABLE

        // "All files access" is what lets `std::fs` enumerate shared storage on
        // Android 11+. Send the user to the system screen and let them retry.
        if (!hasAccess()) {
            main.post { openAllFilesSettings(act) }
            return PICK_NEEDS_ALL_FILES
        }

        synchronized(lock) {
            if (pickActive) return PICK_UNAVAILABLE
            pickActive = true
            resultReady = false
            resultValue = null
        }
        main.post {
            try {
                launcher.launch(treeIntent())
            } catch (error: Throwable) {
                Log.e(TAG, "failed to launch folder picker", error)
                finish(null)
            }
        }
        return PICK_LAUNCHED
    }

    fun onFolderResult(uri: Uri?, cancelled: Boolean) {
        if (cancelled || uri == null) {
            finish(null)
            return
        }
        try {
            activity?.contentResolver?.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION
            )
        } catch (error: Throwable) {
            Log.w(TAG, "could not persist folder grant", error)
        }
        val path = uriToPath(uri)
        if (path == null) {
            finish(null)
            return
        }
        if (hasReadPermission()) {
            finish(path)
            return
        }
        // Pre-Android 11: reading the folder still needs the legacy storage
        // read permission. (On 11+ this is covered by "All files access".)
        pendingPath = path
        val launcher = permissionLauncher
        if (launcher == null) {
            finish(path)
            return
        }
        try {
            launcher.launch(Manifest.permission.READ_EXTERNAL_STORAGE)
        } catch (error: Throwable) {
            Log.w(TAG, "could not request read permission", error)
            finish(path)
        }
    }

    fun onPermissionResult() {
        val path = pendingPath
        pendingPath = null
        finish(path)
    }

    /** Called from Rust. `null` = still waiting, empty = cancelled. */
    @JvmStatic
    fun takeResult(): String? {
        synchronized(lock) {
            if (!resultReady) return null
            resultReady = false
            val value = resultValue
            resultValue = null
            return value ?: ""
        }
    }

    private fun treeIntent(): Intent =
        Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PREFIX_URI_PERMISSION
            )
        }

    private fun openAllFilesSettings(act: Activity) {
        try {
            act.startActivity(
                Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
                    .setData(Uri.parse("package:${act.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            )
        } catch (error: Throwable) {
            Log.w(TAG, "app all-files settings unavailable, using the global list", error)
            try {
                act.startActivity(
                    Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            } catch (fallback: Throwable) {
                Log.e(TAG, "no all-files settings screen", fallback)
            }
        }
    }

    private fun finish(path: String?) {
        synchronized(lock) {
            pickActive = false
            resultReady = true
            resultValue = path
        }
    }

    private fun hasReadPermission(): Boolean {
        // On Android 11+ full access is enforced before the picker opens.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) return true
        val context = activity ?: return false
        return ContextCompat.checkSelfPermission(context, Manifest.permission.READ_EXTERNAL_STORAGE) ==
            PackageManager.PERMISSION_GRANTED
    }

    /**
     * Map `content://.../tree/primary%3AMusic` back to `/storage/emulated/0/Music`.
     * Only filesystem-backed volumes (`primary` and removable `XXXX-XXXX`) are
     * supported; a provider that reports another kind of document id is refused.
     */
    private fun uriToPath(uri: Uri): String? {
        val documentId = try {
            DocumentsContract.getTreeDocumentId(uri)
        } catch (error: Throwable) {
            return null
        }
        val separator = documentId.indexOf(':')
        if (separator < 0) return null
        val volume = documentId.substring(0, separator)
        val relative = documentId.substring(separator + 1)
        val base = when {
            volume.equals("primary", ignoreCase = true) -> "/storage/emulated/0"
            volume.isNotEmpty() -> "/storage/$volume"
            else -> return null
        }
        return if (relative.isEmpty()) base else "$base/$relative"
    }
}
