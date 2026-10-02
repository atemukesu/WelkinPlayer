![Welkin banner](docs/assets/banner.svg)

**[中文](README.md) · [English](README.en.md)**

# Welkin

基于 [Tauri 2](https://v2.tauri.app/) 的跨平台音乐播放器。通过 WebDAV 或本地文件夹读取音乐收藏，把资料、歌单与播放进度跨设备同步，并提供 AMLL 逐字歌词、桌面悬浮歌词与系统媒体控制。

- 仓库：<https://github.com/atemukesu/WelkinPlayer>
- 支持平台：Windows / macOS / Linux / Android
- 界面语言：简体中文、English

## 截图



## 功能特性

- **音乐来源**：同时配置多个 WebDAV 与本地文件夹来源，可指定其中一个作为云端同步位置。
- **歌词**：AMLL 逐字歌词渲染；在线歌词按顺序从 QQ 音乐、网易云音乐、AMLL 歌词库获取；本地同名字幕优先。
- **桌面悬浮歌词**：独立的透明歌词窗口（Android 为系统悬浮窗）。
- **系统媒体控制**：Windows SMTC、macOS Now Playing、Linux MPRIS、Android MediaSession。
- **流式播放与缓存**：Rust 本地回环代理对流媒体做 Range 透传，按网络状态智能缓存，支持手动置顶。
- **跨设备同步**：昵称、歌单、收藏、播放进度等同步到指定来源。
- **音乐管理**：元数据/封面读取与编辑、播放统计与排名。
- **主题**：浅色/深色与多种主题色。

## 技术栈

| 层 | 技术 |
| --- | --- |
| 壳与后端 | Tauri 2（Rust） |
| 前端 | Vue 3 + TypeScript + Vite |
| 状态/i18n | Pinia、vue-i18n |
| 样式 | Tailwind CSS 4 |
| 歌词渲染 | [@applemusic-like-lyrics](https://github.com/amll-dev/applemusic-like-lyrics) |
| 更新 | tauri-plugin-updater（桌面） |

## 环境要求

- Node.js 20+ 与 [pnpm](https://pnpm.io/) 9
- Rust stable（rustup）
- 平台依赖：
  - Windows：WebView2（Win11 自带）
  - macOS：Xcode Command Line Tools
  - Linux：`libwebkit2gtk-4.1-dev`、`libappindicator3-dev`、`librsvg2-dev`、`patchelf`、`libdbus-1-dev`
- 构建 Android 额外需要：JDK 17、Android SDK、Android NDK（r26b）、Android 目标 Rust 组件

## 开发

```bash
pnpm install        # 安装依赖

pnpm tauri dev      # 启动完整桌面应用（Rust + 前端热更新）
pnpm dev            # 仅启动前端（Vite，端口 1420）
```

## 构建

```bash
pnpm build                     # 仅构建前端（含 vue-tsc 类型检查）
pnpm tauri build               # 构建桌面安装包

pnpm tauri android build --apk # 构建 Android APK
```

> Android release 签名通过环境变量注入：`ANDROID_KEYSTORE_PATH`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD`；未配置时回退为 debug 签名。

## 目录结构

```
.
├─ src/                 # Vue 前端
│  ├─ views/            # 页面（设置、来源、歌词编辑等）
│  ├─ stores/           # Pinia 状态
│  └─ lib/              # 通用工具与前端契约
├─ src-tauri/           # Rust 后端
│  ├─ src/commands/     # Tauri 命令（媒体、WebDAV、更新等）
│  ├─ gen/android/      # Android 工程
│  └─ tauri.conf.json   # 应用配置（版本、更新器等）
├─ scripts/             # 构建辅助脚本
└─ .github/workflows/   # CI（发布流程）
```
