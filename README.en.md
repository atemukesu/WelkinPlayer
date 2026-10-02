![Welkin banner](docs/assets/banner.svg)

**[中文](README.md) · [English](README.en.md)**

# Welkin

A cross-platform music player built with [Tauri 2](https://v2.tauri.app/). It reads your music collection over WebDAV or from local folders, syncs your profile, playlists and playback progress across devices, and provides AMLL word-by-word lyrics, a desktop lyrics overlay, and system media controls.

- Repository: <https://github.com/atemukesu/WelkinPlayer>
- Platforms: Windows / macOS / Linux / Android
- Languages: 简体中文, English

## Screenshots

![](/docs/assets/screenshot1.png)
![](/docs/assets/screenshot2.png)
![](/docs/assets/screenshot3.png)
![](/docs/assets/screenshot4.png)

## Features

- **Music sources**: Configure multiple WebDAV and local-folder sources at once; pick one as the cloud sync location.
- **Lyrics**: AMLL word-by-word rendering; online lyrics fetched in order from QQ Music, NetEase Cloud Music and the AMLL lyrics library; same-named local lyrics win.
- **Desktop lyrics overlay**: A separate transparent lyrics window (a system overlay on Android).
- **System media controls**: Windows SMTC, macOS Now Playing, Linux MPRIS, Android MediaSession.
- **Streaming and caching**: A Rust loopback proxy streams audio with Range passthrough, caches smartly based on network state, and supports manual pinning.
- **Cross-device sync**: Nickname, playlists, favorites and playback progress sync to the chosen source.
- **Library management**: Read and edit metadata/cover art; playback statistics and rankings.
- **Themes**: Light/dark and multiple accent colors.

## Tech stack

| Layer | Technology |
| --- | --- |
| Shell & backend | Tauri 2 (Rust) |
| Frontend | Vue 3 + TypeScript + Vite |
| State / i18n | Pinia, vue-i18n |
| Styling | Tailwind CSS 4 |
| Lyrics rendering | [@applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics) |
| Updates | tauri-plugin-updater (desktop) |

## Requirements

- Node.js 20+ and [pnpm](https://pnpm.io/) 9
- Rust stable (rustup)
- Platform dependencies:
  - Windows: WebView2 (bundled with Windows 11)
  - macOS: Xcode Command Line Tools
  - Linux: `libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`, `patchelf`, `libdbus-1-dev`
- For Android builds: JDK 17, Android SDK, Android NDK (r26b), and the Android Rust targets

## Development

```bash
pnpm install        # install dependencies

pnpm tauri dev      # run the full desktop app (Rust + frontend HMR)
pnpm dev            # frontend only (Vite, port 1420)
```

## Build

```bash
pnpm build                     # frontend only (includes vue-tsc type check)
pnpm tauri build               # desktop installers

pnpm tauri android build --apk # Android APK
```

> Android release signing is injected via environment variables: `ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`. It falls back to debug signing when unset.

## Project structure

```
.
├─ src/                 # Vue frontend
│  ├─ views/            # Pages (settings, sources, lyrics editor, ...)
│  ├─ stores/           # Pinia stores
│  └─ lib/              # Shared utilities and frontend contracts
├─ src-tauri/           # Rust backend
│  ├─ src/commands/     # Tauri commands (media, WebDAV, updates, ...)
│  ├─ gen/android/      # Android project
│  └─ tauri.conf.json   # App config (version, updater, ...)
├─ scripts/             # Build helper scripts
└─ .github/workflows/   # CI (release pipeline)
```
