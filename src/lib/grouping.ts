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

/** The grouping key for a track: its trimmed tag, or a shared sentinel. */
export function groupKey(track: Track, kind: GroupKind): string {
  const raw = (kind === "artists" ? track.artist : track.album)?.trim();
  if (raw) return raw;
  return kind === "artists" ? UNKNOWN_ARTIST : UNKNOWN_ALBUM;
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
    const key = groupKey(track, kind);
    const list = map.get(key);
    if (list) list.push(track);
    else map.set(key, [track]);
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
