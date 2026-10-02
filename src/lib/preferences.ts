/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Client-local preferences.
 *
 * These settings are deliberately kept *out* of the synced profile: theme,
 * accent, locale and lyric typography can differ from device to device, so
 * syncing them would let one client overwrite another's choice. They are
 * persisted per client instead (managed by the individual stores and
 * `localStorage`). See `lib/profile.ts` for the cross-device document.
 */

/** Typography for one lyrics renderer (standard or AMLL). */
export interface LyricDisplaySettings {
  lineSize: number;
  translationSize: number;
  lineSpacing: number;
  translate: boolean;
  /** Render the per-character readings (注音 / ruby) that some lyrics carry. */
  ruby: boolean;
  /** CSS font weight (100–900). */
  fontWeight: number;
  /** Ordered `font-family` fallback list; empty falls back to the app font. */
  fontFamilies: string[];
}

/**
 * Typography, colours and behaviour of the floating desktop-lyrics layer.
 *
 * Kept client-local alongside the other lyric display settings: window size,
 * screen and personal taste differ per device, so syncing would fight the user.
 */
export interface DesktopLyricSettings {
  /** Master switch for the floating layer. */
  enabled: boolean;
  /** When locked the window is click-through and ignores the cursor. */
  locked: boolean;
  /** Hide the floating window from the taskbar / dock (desktop only). */
  skipTaskbar: boolean;
  /** Render the translated sub-line under each primary line. */
  translation: boolean;
  /** Draw a word-by-word karaoke sweep instead of plain line highlighting. */
  karaoke: boolean;
  /** Legacy storage field; the desktop overlay only renders the active block. */
  contextLines: number;
  fontSize: number;
  /** Sub-line (translation) size in px. */
  translationSize: number;
  /** CSS font weight (100–900). */
  fontWeight: number;
  /** Ordered `font-family` fallback list; empty uses the app font. */
  fontFamilies: string[];
  /** Idle (non-active) text colour. */
  textColor: string;
  /** Colour of a line while it is being sung. */
  activeColor: string;
  /** Translation sub-line colour. */
  translationColor: string;
  /** Whole-layer opacity, 0–100. */
  opacity: number;
  /** Draw a 1px outline around the text. */
  stroke: boolean;
  /** Outline colour used when `stroke` is enabled. */
  strokeColor: string;
}

export const DEFAULT_DESKTOP_LYRIC: DesktopLyricSettings = {
  enabled: false,
  locked: false,
  skipTaskbar: true,
  translation: true,
  karaoke: true,
  // Kept in the type and storage normalizer for backwards compatibility;
  // the single-line overlay no longer renders surrounding context.
  contextLines: 0,
  fontSize: 28,
  translationSize: 20,
  fontWeight: 700,
  fontFamilies: [],
  textColor: "#f5f5f5",
  activeColor: "#f0a500",
  translationColor: "#b9bec8",
  opacity: 100,
  stroke: false,
  strokeColor: "#000000",
};

/** Where a lyric provider may be selected on the settings page. */
export type LyricProvider = "qq" | "local" | "netease" | "amll";

/** All lyric providers in their default priority order (highest first). */
export const LYRIC_PROVIDERS: LyricProvider[] = ["local", "amll", "qq", "netease"];

export const DEFAULT_CLASSIC_DISPLAY: LyricDisplaySettings = {
  lineSize: 38,
  translationSize: 26,
  lineSpacing: 24,
  translate: true,
  ruby: true,
  fontWeight: 600,
  fontFamilies: [],
};

export const DEFAULT_AMLL_DISPLAY: LyricDisplaySettings = {
  lineSize: 38,
  translationSize: 26,
  lineSpacing: 24,
  translate: true,
  ruby: true,
  fontWeight: 400,
  fontFamilies: [],
};

/** Fold a stored provider list into the canonical set, dropping duplicates. */
export function normalizeLyricProviders(value: unknown, fallback: LyricProvider[] = LYRIC_PROVIDERS): LyricProvider[] {
  if (!Array.isArray(value)) return [...fallback];
  const seen = new Set<LyricProvider>();
  const result: LyricProvider[] = [];
  for (const item of value) {
    if (typeof item === "string" && (LYRIC_PROVIDERS as string[]).includes(item)) {
      const provider = item as LyricProvider;
      if (!seen.has(provider)) {
        seen.add(provider);
        result.push(provider);
      }
    }
  }
  for (const provider of LYRIC_PROVIDERS) {
    if (!seen.has(provider)) result.push(provider);
  }
  return result.length > 0 ? result : [...fallback];
}
