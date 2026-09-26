import type { Accent, Theme } from "./app";

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

/** Lyrics-related preferences, mirrored from the lyrics store. */
export interface LyricsPreferences {
  /** Whether lyric fetching is enabled at all. */
  enabled: boolean;
  /**
   * Ordered list of lyric providers, highest priority first. The player walks
   * this list in order and uses the first provider that returns lyrics.
   */
  providers: LyricProvider[];
  /** Render the player page with the AMLL (Apple Music-like Lyrics) component. */
  useAmll: boolean;
  /** Typography for the standard player, used when {@link useAmll} is false. */
  classic: LyricDisplaySettings;
  /** Typography for the AMLL player, used when {@link useAmll} is true. */
  amll: LyricDisplaySettings;
}

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

/** Look-and-feel preferences that travel with the profile. */
export interface AppearancePreferences {
  theme: Theme;
  accent: Accent;
  locale: "zh-CN" | "en";
}

/** A user-curated list of remote track paths. */
export interface Playlist {
  id: string;
  name: string;
  tracks: string[];
  /**
   * @deprecated Legacy uploaded cover embedded as a compressed data URL. New
   * covers are uploaded as standalone files and referenced by {@link coverFile};
   * this field is only kept so existing profiles render until they migrate.
   */
  cover?: string;
  /**
   * Remote file name of an uploaded cover (e.g. `welkin-cover-<sha>.jpg`). The
   * image itself lives on WebDAV and in the local cover cache, never in the
   * profile document.
   */
  coverFile?: string;
  /** Remote track path whose cover/icon is used as the playlist cover. */
  coverTrack?: string;
}

/**
 * The complete cross-device profile. Serialized as one JSON document so a
 * single WebDAV `GET` restores everything.
 */
export interface Profile {
  version: number;
  /** Whether the first-run wizard has been completed. */
  initialized: boolean;
  nickname: string;
  appearance: AppearancePreferences;
  lyrics: LyricsPreferences;
  /** Remote track path -> number of times playback started. */
  playCounts: Record<string, number>;
  /** Remote track paths the user marked as favorite. */
  favorites: string[];
  /** Remote track paths whose lyrics are intentionally suppressed. */
  disabledLyrics: string[];
  /** Most recently played remote track paths, newest first. */
  recent: string[];
  /**
   * @deprecated Legacy resume point. Superseded by the playback store
   * (`playback.json` / `welkin-playback.json`); kept only to migrate existing
   * documents on first launch.
   */
  lastTrack?: string;
  /**
   * @deprecated Legacy playback position (seconds). See {@link lastTrack}.
   */
  lastPosition?: number;
  playlists: Playlist[];
  /** Last local modification time (ms). */
  updatedAt: number;
}

export const PROFILE_VERSION = 1;

export function createDefaultProfile(seed?: Partial<AppearancePreferences & LyricsPreferences>): Profile {
  return {
    version: PROFILE_VERSION,
    initialized: false,
    nickname: "",
    appearance: {
      theme: seed?.theme ?? "light",
      accent: seed?.accent ?? "amber",
      locale: seed?.locale ?? "zh-CN",
    },
    lyrics: {
      enabled: seed?.enabled ?? true,
      providers: seed?.providers ? [...seed.providers] : [...LYRIC_PROVIDERS],
      useAmll: seed?.useAmll ?? false,
      classic: seed?.classic ? { ...seed.classic } : { ...DEFAULT_CLASSIC_DISPLAY },
      amll: seed?.amll ? { ...seed.amll } : { ...DEFAULT_AMLL_DISPLAY },
    },
    playCounts: {},
    favorites: [],
    disabledLyrics: [],
    recent: [],
    playlists: [],
    updatedAt: 0,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** Parse one renderer's typography, filling missing/malformed fields. */
function parseDisplaySettings(value: unknown, fallback: LyricDisplaySettings): LyricDisplaySettings {
  const data = isRecord(value) ? value : {};
  return {
    lineSize: typeof data.lineSize === "number" && Number.isFinite(data.lineSize) ? data.lineSize : fallback.lineSize,
    translationSize: typeof data.translationSize === "number" && Number.isFinite(data.translationSize) ? data.translationSize : fallback.translationSize,
    lineSpacing: typeof data.lineSpacing === "number" && Number.isFinite(data.lineSpacing) ? data.lineSpacing : fallback.lineSpacing,
    translate: typeof data.translate === "boolean" ? data.translate : fallback.translate,
    ruby: typeof data.ruby === "boolean" ? data.ruby : fallback.ruby,
    fontWeight: typeof data.fontWeight === "number" && Number.isFinite(data.fontWeight) ? data.fontWeight : fallback.fontWeight,
    fontFamilies: Array.isArray(data.fontFamilies)
      ? data.fontFamilies.filter((item): item is string => typeof item === "string")
      : [...fallback.fontFamilies],
  };
}

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

/**
 * Legacy flat lyrics fields predate the per-mode split; seed both renderers
 * from them so an upgrade keeps the user's existing size/spacing.
 */
function legacyDisplay(fallback: LyricDisplaySettings, lyrics: Record<string, unknown>): LyricDisplaySettings {
  return {
    ...fallback,
    lineSize: typeof lyrics.lineSize === "number" ? lyrics.lineSize : fallback.lineSize,
    translationSize: typeof lyrics.translationSize === "number" ? lyrics.translationSize : fallback.translationSize,
    lineSpacing: typeof lyrics.lineSpacing === "number" ? lyrics.lineSpacing : fallback.lineSpacing,
    translate: typeof lyrics.translate === "boolean" ? lyrics.translate : fallback.translate,
  };
}

/**
 * Parse a stored profile document, filling in anything missing or malformed so
 * a partially-written / older document never breaks startup.
 */
export function parseProfile(raw: string | null | undefined, fallback: Profile): Profile {
  if (!raw) return fallback;
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return fallback;
  }
  if (!isRecord(data)) return fallback;

  const appearance = isRecord(data.appearance) ? data.appearance : {};
  const lyrics = isRecord(data.lyrics) ? data.lyrics : {};
  const playCounts: Record<string, number> = {};
  if (isRecord(data.playCounts)) {
    for (const [key, value] of Object.entries(data.playCounts)) {
      if (typeof value === "number" && Number.isFinite(value) && value > 0) {
        playCounts[key] = Math.floor(value);
      }
    }
  }

  const favorites = Array.isArray(data.favorites)
    ? data.favorites.filter((item): item is string => typeof item === "string")
    : [];

  const disabledLyrics = Array.isArray(data.disabledLyrics)
    ? data.disabledLyrics.filter((item): item is string => typeof item === "string")
    : [];

  const recent = Array.isArray(data.recent)
    ? data.recent.filter((item): item is string => typeof item === "string")
    : [];

  const playlists: Playlist[] = Array.isArray(data.playlists)
    ? data.playlists.flatMap((item) => {
        if (!isRecord(item) || typeof item.name !== "string") return [];
        const tracks = Array.isArray(item.tracks)
          ? item.tracks.filter((track): track is string => typeof track === "string")
          : [];
        return [{
          id: typeof item.id === "string" ? item.id : createId(),
          name: item.name,
          tracks,
          cover: typeof item.cover === "string" ? item.cover : undefined,
          coverFile: typeof item.coverFile === "string" ? item.coverFile : undefined,
          coverTrack: typeof item.coverTrack === "string" ? item.coverTrack : undefined,
        }];
      })
    : [];

  return {
    version: PROFILE_VERSION,
    initialized: data.initialized === true,
    nickname: typeof data.nickname === "string" ? data.nickname : fallback.nickname,
    appearance: {
      theme: appearance.theme === "dark" ? "dark" : appearance.theme === "light" ? "light" : fallback.appearance.theme,
      accent: typeof appearance.accent === "string" ? (appearance.accent as Accent) : fallback.appearance.accent,
      locale: appearance.locale === "en" ? "en" : appearance.locale === "zh-CN" ? "zh-CN" : fallback.appearance.locale,
    },
    lyrics: {
      // Legacy documents stored a flat `source` enum; migrate "disabled" so an
      // upgrade keeps the user's opt-out.
      enabled:
        typeof lyrics.enabled === "boolean"
          ? lyrics.enabled
          : lyrics.source === "disabled"
            ? false
            : fallback.lyrics.enabled,
      providers: normalizeLyricProviders(lyrics.providers, fallback.lyrics.providers),
      useAmll: typeof lyrics.useAmll === "boolean" ? lyrics.useAmll : fallback.lyrics.useAmll,
      classic: parseDisplaySettings(lyrics.classic, legacyDisplay(fallback.lyrics.classic, lyrics)),
      amll: parseDisplaySettings(lyrics.amll, legacyDisplay(fallback.lyrics.amll, lyrics)),
    },
    playCounts,
    favorites,
    disabledLyrics,
    recent,
    lastTrack: typeof data.lastTrack === "string" ? data.lastTrack : undefined,
    lastPosition: typeof data.lastPosition === "number" && Number.isFinite(data.lastPosition) ? Math.max(0, data.lastPosition) : undefined,
    playlists,
    updatedAt: typeof data.updatedAt === "number" ? data.updatedAt : 0,
  };
}

/** RFC 4122 id, with a timestamp fallback for older webviews. */
export function createId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `id-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}
