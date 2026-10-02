//! In-app updates.
//!
//! Desktop uses Tauri's official updater: the release workflow signs every
//! installer with the developer key and publishes a `latest.json` manifest as a
//! GitHub release asset, so the backend verifies each download against the
//! embedded public key before installing it.
//!
//! Android is deliberately different. Installing an APK from inside the app
//! needs the install-packages permission and a user-facing package installer,
//! so instead the newest GitHub release is queried directly and the webview
//! offers the `.apk` asset as a normal browser download the user installs
//! themselves. Both paths return the same [`UpdateInfo`] so the settings screen
//! can render the release notes identically.

use serde::Serialize;
use tauri::AppHandle;

use crate::error::AppError;

/// GitHub repository whose releases carry the update artifacts (Android path).
#[cfg(target_os = "android")]
pub const REPO: &str = "atemukesu/WelkinPlayer";

/// A discovered update, normalized across the desktop and Android paths.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Whether a newer version than the running one was found.
    pub available: bool,
    /// The version currently running.
    pub current_version: String,
    /// The newer version, when one is available.
    pub version: Option<String>,
    /// Release notes / changelog body, when the release carries one.
    pub notes: Option<String>,
    /// Publish date (RFC 3339 on Android, the updater's format on desktop).
    pub date: Option<String>,
    /// Direct download URL for platforms that install outside the app (Android).
    pub download_url: Option<String>,
}

impl UpdateInfo {
    /// The "nothing to do" result, shared by both platforms.
    fn up_to_date(current_version: String) -> Self {
        Self {
            available: false,
            current_version,
            version: None,
            notes: None,
            date: None,
            download_url: None,
        }
    }
}

/// Progress streamed to the webview while a desktop update downloads.
///
/// The shape mirrors the official updater example so the frontend can key off
/// `event.event` (`Started` / `Progress` / `Finished`).
#[cfg(desktop)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DownloadEvent {
    #[serde(rename_all = "camelCase")]
    Started { content_length: Option<u64> },
    #[serde(rename_all = "camelCase")]
    Progress { chunk_length: usize },
    Finished,
}

/// Desktop update backend, backed by `tauri-plugin-updater`.
#[cfg(desktop)]
pub mod desktop {
    use std::sync::Mutex;

    use tauri::{ipc::Channel, AppHandle};
    use tauri_plugin_updater::{Update, UpdaterExt};

    use super::{DownloadEvent, UpdateInfo};
    use crate::error::AppError;

    /// The update found by [`check`], held between the check and install calls.
    ///
    /// The plugin's `Update` value owns the verified download metadata, so it
    /// cannot be re-derived frontend-side; the backend keeps it until the user
    /// confirms the install.
    #[derive(Default)]
    pub struct PendingUpdate(pub Mutex<Option<Update>>);

    /// Ask the configured endpoint for an update, remembering it for install.
    pub async fn check(app: &AppHandle, pending: &PendingUpdate) -> Result<UpdateInfo, AppError> {
        let update = app
            .updater()
            .map_err(|error| AppError::Other(format!("更新器初始化失败：{error}")))?
            .check()
            .await
            .map_err(|error| AppError::Other(format!("检查更新失败：{error}")))?;

        match update {
            Some(update) => {
                let info = UpdateInfo {
                    available: true,
                    current_version: update.current_version.clone(),
                    version: Some(update.version.clone()),
                    notes: update
                        .body
                        .clone()
                        .filter(|text| !text.trim().is_empty()),
                    date: update.date.map(|date| date.to_string()),
                    download_url: Some(update.download_url.to_string()),
                };
                *pending.0.lock().unwrap() = Some(update);
                Ok(info)
            }
            None => Ok(UpdateInfo::up_to_date(
                app.package_info().version.to_string(),
            )),
        }
    }

    /// Download and install the remembered update, streaming progress.
    ///
    /// On Windows the installer exits the app on its own; elsewhere the app is
    /// relaunched here so the freshly installed build takes over.
    pub async fn install(
        app: &AppHandle,
        pending: &PendingUpdate,
        on_event: Channel<DownloadEvent>,
    ) -> Result<(), AppError> {
        let update = { pending.0.lock().unwrap().take() };
        let Some(update) = update else {
            return Err(AppError::Other("没有可安装的更新".to_string()));
        };

        let mut started = false;
        update
            .download_and_install(
                |chunk_length, content_length| {
                    if !started {
                        let _ = on_event.send(DownloadEvent::Started { content_length });
                        started = true;
                    }
                    let _ = on_event.send(DownloadEvent::Progress { chunk_length });
                },
                || {},
            )
            .await
            .map_err(|error| AppError::Other(format!("下载或安装更新失败：{error}")))?;

        let _ = on_event.send(DownloadEvent::Finished);
        app.restart()
    }
}

/// Android update backend, backed by the GitHub Releases API.
#[cfg(target_os = "android")]
pub mod android {
    use serde::Deserialize;
    use tauri::AppHandle;

    use super::{UpdateInfo, REPO};
    use crate::error::AppError;

    /// The subset of a GitHub release the updater cares about.
    #[derive(Debug, Deserialize)]
    struct Release {
        tag_name: String,
        body: Option<String>,
        published_at: Option<String>,
        #[serde(default)]
        assets: Vec<Asset>,
    }

    /// One downloadable file attached to a release.
    #[derive(Debug, Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }

    /// Ask GitHub for the newest release and compare it with the running build.
    pub async fn check(app: &AppHandle) -> Result<UpdateInfo, AppError> {
        let current = app.package_info().version.to_string();
        let release = tauri::async_runtime::spawn_blocking(fetch_latest)
            .await
            .map_err(|error| AppError::Other(format!("检查更新任务失败：{error}")))??;

        let latest = release.tag_name.trim_start_matches('v').to_string();
        let available = match (
            semver::Version::parse(&latest),
            semver::Version::parse(&current),
        ) {
            (Ok(latest), Ok(current)) => latest > current,
            // Fall back to a raw comparison when either side is not strict SemVer.
            _ => latest != current,
        };

        if !available {
            return Ok(UpdateInfo::up_to_date(current));
        }

        // Prefer the universal APK when the release carries more than one.
        let download_url = release
            .assets
            .iter()
            .find(|asset| asset.name.to_ascii_lowercase().ends_with(".apk"))
            .map(|asset| asset.browser_download_url.clone());

        Ok(UpdateInfo {
            available: true,
            current_version: current,
            version: Some(latest),
            notes: release.body.filter(|text| !text.trim().is_empty()),
            date: release.published_at,
            download_url,
        })
    }

    /// Blocking GitHub API call, run off the async runtime.
    fn fetch_latest() -> Result<Release, AppError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(AppError::from)?;

        let response = client
            .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
            .header("User-Agent", "WelkinPlayer")
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(AppError::from)?
            .error_for_status()
            .map_err(AppError::from)?;

        response.json::<Release>().map_err(AppError::from)
    }
}

/// Check for an update on desktop.
#[cfg(desktop)]
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    pending: tauri::State<'_, desktop::PendingUpdate>,
) -> Result<UpdateInfo, AppError> {
    desktop::check(&app, &pending).await
}

/// Download and install the pending desktop update, reporting progress.
#[cfg(desktop)]
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    pending: tauri::State<'_, desktop::PendingUpdate>,
    on_event: tauri::ipc::Channel<DownloadEvent>,
) -> Result<(), AppError> {
    desktop::install(&app, &pending, on_event).await
}

/// Check for an update on Android.
#[cfg(target_os = "android")]
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, AppError> {
    android::check(&app).await
}
