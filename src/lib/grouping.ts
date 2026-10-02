/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import type { Track } from "../stores/player";

/** The two library classifications available from the navigation. */
export type GroupKind = "artists" | "albums";

/**
 * Sentinel group keys for tracks whose artist/album tag is missing. They are
 * stable, non-empty strings so callers can use truthiness to detect an active
 * classification drill-down and so the keys never collide with real names.
 */
export const UNKNOWN_ARTIST = "\u0000welkin:unknown-artist";
export const UNKNOWN_ALBUM = "\u0000welkin:unknown-album";

/**
 * Separators used between multiple artists in a single tag. Covers ASCII and
 * fullwidth commas, the CJK enumeration comma and semicolons, so collaborations
 * like "A, B" or "A、B" land under both artists instead of one combined name.
 */
const ARTIST_SEPARATORS = /[,，、;；]+/;

/** Splits a raw artist tag into individual names, preserving order. */
export function splitArtists(artist: string): string[] {
  const parts = artist.split(ARTIST_SEPARATORS).map((part) => part.trim()).filter(Boolean);
  return parts.length > 0 ? parts : [artist.trim()];
}

/** The first named artist of a tag, used as the target when navigating from a track. */
export function primaryArtist(artist: string): string {
  return splitArtists(artist)[0] ?? "";
}

/** The album key used to open an album collection; empty tags have no collection to open. */
export function albumKey(album: string): string {
  return album.trim();
}

/** All grouping keys a track belongs to; a collaboration track can appear under several artists. */
export function groupKeys(track: Track, kind: GroupKind): string[] {
  if (kind === "artists") {
    const keys = splitArtists(track.artist ?? "").map((name) => name || UNKNOWN_ARTIST);
    return [...new Set(keys)];
  }
  const album = (track.album ?? "").trim();
  return [album || UNKNOWN_ALBUM];
}

/** The primary grouping key for a track: its first trimmed tag, or a shared sentinel. */
export function groupKey(track: Track, kind: GroupKind): string {
  return groupKeys(track, kind)[0];
}

/** True for the sentinel keys above, so the UI can localize "Unknown". */
export function isUnknownGroup(key: string): boolean {
  return key === UNKNOWN_ARTIST || key === UNKNOWN_ALBUM;
}

export interface TrackGroup {
  key: string;
  tracks: Track[];
}

/** Group tracks by artist or album, sorted case-insensitively, unknown last. */
export function groupTracks(tracks: Track[], kind: GroupKind): TrackGroup[] {
  const map = new Map<string, Track[]>();
  for (const track of tracks) {
    for (const key of groupKeys(track, kind)) {
      const list = map.get(key);
      if (list) list.push(track);
      else map.set(key, [track]);
    }
  }
  return [...map.entries()]
    .map(([key, list]) => ({ key, tracks: list }))
    .sort((a, b) => {
      const unknownA = isUnknownGroup(a.key);
      const unknownB = isUnknownGroup(b.key);
      if (unknownA !== unknownB) return unknownA ? 1 : -1;
      return a.key.localeCompare(b.key, undefined, { sensitivity: "base" });
    });
}
