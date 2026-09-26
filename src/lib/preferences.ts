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

/** Where a lyric provider may be selected on the settings page. */
export type LyricProvider = "qq" | "local" | "netease" | "amll";

/** All lyric providers in their default priority order (highest first). */
export const LYRIC_PROVIDERS: LyricProvider[] = ["local", "amll", "qq", "netease"];

export const DEFAULT_CLASSIC_DISPLAY: LyricDisplaySettings = {
  lineSize: 24,
  translationSize: 18,
  lineSpacing: 24,
  translate: true,
  ruby: true,
  fontWeight: 600,
  fontFamilies: [],
};

export const DEFAULT_AMLL_DISPLAY: LyricDisplaySettings = {
  lineSize: 24,
  translationSize: 18,
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
