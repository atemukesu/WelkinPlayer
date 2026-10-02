/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Shared contract for the floating desktop-lyrics layer.
 *
 * The main window is the single source of truth: it slices the parsed lyrics
 * and the live playhead into the small documents below and ships them to the
 * backend, which forwards them to whichever floating renderer is running
 * (a transparent Tauri window on desktop, a WebView overlay on Android).
 *
 * Both renderers consume the exact same shapes, so the field names here are
 * mirrored by the Rust `serde(rename_all = "camelCase")` structs.
 */

/** One karaoke word, timing in milliseconds relative to the track start. */
export interface DesktopLyricWord {
  text: string;
  start: number;
  end: number;
}

/** One lyric line, pre-flattened so the renderer needs no parser. */
export interface DesktopLyricLine {
  text: string;
  translation: string;
  /** Romanisation, when the source carries one. */
  roman: string;
  words: DesktopLyricWord[];
  /** Background vocal. */
  isBG: boolean;
  /** Duet counter-line (the second voice). */
  isDuet: boolean;
}

/** Full snapshot, sent only when the track / lyrics / settings change. */
export interface DesktopLyricLoadPayload {
  title: string;
  artist: string;
  lines: DesktopLyricLine[];
  /** Style document forwarded verbatim to the renderer. */
  settings: Record<string, unknown>;
}

/**
 * High-frequency playhead document. Kept deliberately tiny so pushing it at
 * ~30 Hz stays cheap over IPC / JNI.
 */
export interface DesktopLyricTickPayload {
  positionMs: number;
  playing: boolean;
  /** Primary line index, or -1. */
  activeIndex: number;
  /** Every line whose window overlaps the playhead (BG vocals + duet lines). */
  activeIndices: number[];
  /** Whether the layer should currently be painted at all. */
  visible: boolean;
}

/** True when running inside the Tauri shell. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** True on Android (Tauri mobile), where the floating layer is a WebView overlay. */
export function isAndroid(): boolean {
  return typeof navigator !== "undefined" && /android/i.test(navigator.userAgent);
}

/** True on the desktop window managers (everything but Android/iOS). */
export function isDesktop(): boolean {
  return isTauri() && !isAndroid() && !/iphone|ipad|ipod/i.test(navigator.userAgent);
}

/** Label of the desktop floating window, shared by JS and Rust. */
export const DESKTOP_LYRIC_WINDOW_LABEL = "desktop-lyrics";

/** LocalStorage key holding the last floating-window geometry. */
export const DESKTOP_LYRIC_GEOMETRY_KEY = "welkin-desktop-lyric-geometry";
