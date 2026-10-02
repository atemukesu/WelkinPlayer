/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Cross-device profile document.
 *
 * The profile holds only data that should travel between devices: nickname,
 * play counts, favorites, playlists and the deprecated playback migration
 * fields. Client-specific preferences (theme, accent, locale, lyric typography,
 * …) are intentionally *not* stored here — they live in each device's local
 * storage so clients never overwrite one another. See `lib/preferences.ts`.
 */

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

export const PROFILE_VERSION = 3;

export function createDefaultProfile(): Profile {
  return {
    version: PROFILE_VERSION,
    initialized: false,
    nickname: "",
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
