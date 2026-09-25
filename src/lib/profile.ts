import type { Accent, Theme } from "./app";

/** Lyrics-related preferences, mirrored from the lyrics store. */
export interface LyricsPreferences {
  source: "local" | "disabled";
  lineSize: number;
  translationSize: number;
  lineSpacing: number;
  translate: boolean;
}

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
  /** Uploaded cover, stored as a compressed square data URL. */
  cover?: string;
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
  /** Most recently played remote track paths, newest first. */
  recent: string[];
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
      source: seed?.source ?? "local",
      lineSize: seed?.lineSize ?? 24,
      translationSize: seed?.translationSize ?? 18,
      lineSpacing: seed?.lineSpacing ?? 24,
      translate: seed?.translate ?? true,
    },
    playCounts: {},
    favorites: [],
    recent: [],
    playlists: [],
    updatedAt: 0,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
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
      source: lyrics.source === "disabled" ? "disabled" : "local",
      lineSize: typeof lyrics.lineSize === "number" ? lyrics.lineSize : fallback.lyrics.lineSize,
      translationSize: typeof lyrics.translationSize === "number" ? lyrics.translationSize : fallback.lyrics.translationSize,
      lineSpacing: typeof lyrics.lineSpacing === "number" ? lyrics.lineSpacing : fallback.lyrics.lineSpacing,
      translate: typeof lyrics.translate === "boolean" ? lyrics.translate : fallback.lyrics.translate,
    },
    playCounts,
    favorites,
    recent,
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
