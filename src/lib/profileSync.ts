/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Cross-device profile synchronization primitives.
 *
 * Every user edit is recorded as a replayable operation. Before a write the
 * engine fetches the newest cloud document and re-applies those operations on
 * top of it, so edits from another device survive instead of being overwritten
 * by a whole-document `PUT`. Only incompatible edits on the same data produce a
 * conflict, which the UI resolves by asking the user which version to keep.
 */

import type { Playlist, Profile, SmartTransitionMeta } from "./profile";

/** A mutable subset of playlist fields, matching `updatePlaylist`. */
export interface PlaylistPatch {
  name?: string;
  tracks?: string[];
  cover?: string;
  coverFile?: string;
  coverTrack?: string;
  smartTransition?: SmartTransitionMeta;
  /** Explicit deletions cannot be expressed with optional fields alone. */
  clearCover?: boolean;
  clearCoverTrack?: boolean;
  clearSmartTransition?: boolean;
}

/** One user edit, replayed on the cloud document before writing it back. */
export type ProfileOp =
  | { type: "setNickname"; value: string }
  | { type: "setInitialized"; value: boolean }
  | { type: "recordPlay"; key: string }
  | { type: "setFavorite"; key: string; value: boolean }
  | { type: "setLyricsDisabled"; key: string; value: boolean }
  | { type: "clearLegacyPlayback" }
  | { type: "createPlaylist"; playlist: Playlist }
  | { type: "renamePlaylist"; id: string; name: string }
  | { type: "deletePlaylist"; id: string }
  | { type: "addTrack"; id: string; key: string }
  | { type: "removeTrack"; id: string; key: string }
  | { type: "moveTrack"; id: string; key: string; direction: "up" | "down" }
  | { type: "updatePlaylist"; id: string; patch: PlaylistPatch }
  | { type: "markSmartTransitionStale"; id: string };

/** Fields inside a playlist whose value cannot be merged automatically. */
export type ProfileConflictKind =
  | "playlistTracks"
  | "playlistName"
  | "playlistCover"
  | "playlistDeleted"
  | "nickname"
  | "favorite"
  | "lyrics"
  | "remoteChanged";

/** A single incompatible change shown in the conflict dialog. */
export interface ProfileConflict {
  id: string;
  kind: ProfileConflictKind;
  /** Playlist name or track key for display. */
  name: string;
  /** Human-readable local value, e.g. a track count. */
  local?: string;
  /** Human-readable cloud value. */
  remote?: string;
}

/** The exact amount of state needed to resume pending edits after a restart. */
export interface PersistedProfileSync {
  ops: ProfileOp[];
  baseRevision: string | null;
  baseProfile: Profile;
}

const STORAGE_KEY = "welkin-profile-sync";
const RECENT_LIMIT = 12;

/** Deep-clone a profile document through JSON (the wire format itself). */
export function cloneProfile(value: Profile): Profile {
  return JSON.parse(JSON.stringify(value)) as Profile;
}

function findPlaylist(profile: Profile, id: string): Playlist | undefined {
  return profile.playlists.find((item) => item.id === id);
}

function sameValue(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function shortKey(key: string): string {
  return key.split(/[\\/]/).filter(Boolean).pop() ?? key;
}

function pushConflict(
  conflicts: ProfileConflict[],
  conflict: ProfileConflict,
): void {
  if (conflicts.some((item) => item.id === conflict.id)) return;
  conflicts.push(conflict);
}

/**
 * Replay locally-recorded operations on the newest cloud profile.
 *
 * `base` is the profile the operations were originally derived from; it is only
 * used to distinguish "changed remotely" from "changed locally". Conflicting
 * fields are left at their cloud values and reported in `conflicts`; the result
 * is only written when that list is empty.
 */
export function replayProfileOps(
  base: Profile,
  remote: Profile,
  ops: ProfileOp[],
): { profile: Profile; conflicts: ProfileConflict[] } {
  const profile = cloneProfile(remote);
  const conflicts: ProfileConflict[] = [];

  for (const op of ops) {
    switch (op.type) {
      case "setNickname": {
        if (profile.nickname !== op.value) {
          if (profile.nickname !== base.nickname) {
            pushConflict(conflicts, {
              id: "nickname",
              kind: "nickname",
              name: "nickname",
              local: op.value,
              remote: profile.nickname,
            });
          } else {
            profile.nickname = op.value;
          }
        }
        break;
      }
      case "setInitialized": {
        profile.initialized = op.value;
        break;
      }
      case "recordPlay": {
        profile.playCounts[op.key] = (profile.playCounts[op.key] ?? 0) + 1;
        profile.recent = [
          op.key,
          ...profile.recent.filter((item) => item !== op.key),
        ].slice(0, RECENT_LIMIT);
        break;
      }
      case "setFavorite": {
        const has = profile.favorites.includes(op.key);
        if (has !== op.value) {
          const baseHas = base.favorites.includes(op.key);
          if (has !== baseHas) {
            pushConflict(conflicts, {
              id: `favorite:${op.key}`,
              kind: "favorite",
              name: shortKey(op.key),
              local: op.value ? "on" : "off",
              remote: has ? "on" : "off",
            });
          } else if (op.value) {
            profile.favorites.push(op.key);
          } else {
            profile.favorites = profile.favorites.filter((item) => item !== op.key);
          }
        }
        break;
      }
      case "setLyricsDisabled": {
        const has = profile.disabledLyrics.includes(op.key);
        if (has !== op.value) {
          const baseHas = base.disabledLyrics.includes(op.key);
          if (has !== baseHas) {
            pushConflict(conflicts, {
              id: `lyrics:${op.key}`,
              kind: "lyrics",
              name: shortKey(op.key),
              local: op.value ? "on" : "off",
              remote: has ? "on" : "off",
            });
          } else if (op.value) {
            profile.disabledLyrics.push(op.key);
          } else {
            profile.disabledLyrics = profile.disabledLyrics.filter((item) => item !== op.key);
          }
        }
        break;
      }
      case "clearLegacyPlayback": {
        delete profile.lastTrack;
        delete profile.lastPosition;
        break;
      }
      case "createPlaylist": {
        const existing = findPlaylist(profile, op.playlist.id);
        if (!existing) profile.playlists.push(JSON.parse(JSON.stringify(op.playlist)) as Playlist);
        else if (!sameValue(existing, op.playlist)) {
          pushConflict(conflicts, {
            id: `playlist:${op.playlist.id}`,
            kind: "playlistDeleted",
            name: op.playlist.name,
          });
        }
        break;
      }
      case "renamePlaylist": {
        const playlist = findPlaylist(profile, op.id);
        if (!playlist) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}`,
            kind: "playlistDeleted",
            name: op.name,
          });
          break;
        }
        if (playlist.name === op.name) break;
        const baseName = findPlaylist(base, op.id)?.name;
        if (playlist.name !== baseName) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}:name`,
            kind: "playlistName",
            name: playlist.name,
            local: op.name,
            remote: playlist.name,
          });
        } else {
          playlist.name = op.name;
        }
        break;
      }
      case "deletePlaylist": {
        const playlist = findPlaylist(profile, op.id);
        if (!playlist) break;
        const basePlaylist = findPlaylist(base, op.id);
        if (basePlaylist && !sameValue(playlist, basePlaylist)) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}`,
            kind: "playlistDeleted",
            name: playlist.name,
          });
          break;
        }
        profile.playlists = profile.playlists.filter((item) => item.id !== op.id);
        break;
      }
      case "addTrack": {
        const playlist = findPlaylist(profile, op.id);
        if (!playlist) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}`,
            kind: "playlistDeleted",
            name: op.id,
          });
          break;
        }
        if (!playlist.tracks.includes(op.key)) {
          playlist.tracks.push(op.key);
          markStale(playlist);
        }
        break;
      }
      case "removeTrack": {
        const playlist = findPlaylist(profile, op.id);
        if (!playlist) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}`,
            kind: "playlistDeleted",
            name: op.id,
          });
          break;
        }
        if (playlist.tracks.includes(op.key)) {
          playlist.tracks = playlist.tracks.filter((item) => item !== op.key);
          markStale(playlist);
        }
        break;
      }
      case "moveTrack": {
        const playlist = findPlaylist(profile, op.id);
        if (!playlist) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}`,
            kind: "playlistDeleted",
            name: op.id,
          });
          break;
        }
        const index = playlist.tracks.indexOf(op.key);
        if (index < 0) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}:tracks`,
            kind: "playlistTracks",
            name: playlist.name,
            local: shortKey(op.key),
            remote: String(playlist.tracks.length),
          });
          break;
        }
        const target = op.direction === "up" ? index - 1 : index + 1;
        if (target < 0 || target >= playlist.tracks.length) break;
        const next = [...playlist.tracks];
        [next[index], next[target]] = [next[target], next[index]];
        playlist.tracks = next;
        markStale(playlist);
        break;
      }
      case "updatePlaylist": {
        const playlist = findPlaylist(profile, op.id);
        const basePlaylist = findPlaylist(base, op.id);
        if (!playlist) {
          pushConflict(conflicts, {
            id: `playlist:${op.id}`,
            kind: "playlistDeleted",
            name: op.id,
          });
          break;
        }
        const patch = op.patch;
        /**
         * A patch from the editor carries every field, even unchanged ones.
         * Only treat a field as a local edit when it differs from the base,
         * otherwise an unrelated remote change would raise a false conflict.
         */
        const localChanged = (value: unknown, baseValue: unknown): boolean =>
          basePlaylist ? !sameValue(value, baseValue) : true;
        const remoteChanged = (remoteValue: unknown, baseValue: unknown): boolean =>
          !!basePlaylist && !sameValue(remoteValue, baseValue);

        if (patch.name !== undefined && playlist.name !== patch.name) {
          const changedLocally = localChanged(patch.name, basePlaylist?.name);
          const changedRemotely = remoteChanged(playlist.name, basePlaylist?.name);
          if (changedLocally && changedRemotely) {
            pushConflict(conflicts, {
              id: `playlist:${op.id}:name`,
              kind: "playlistName",
              name: playlist.name,
              local: patch.name,
              remote: playlist.name,
            });
          } else if (changedLocally) {
            playlist.name = patch.name;
          }
        }

        if (patch.tracks !== undefined && !sameValue(playlist.tracks, patch.tracks)) {
          const changedLocally = localChanged(patch.tracks, basePlaylist?.tracks);
          const changedRemotely = remoteChanged(playlist.tracks, basePlaylist?.tracks);
          if (changedLocally && changedRemotely) {
            pushConflict(conflicts, {
              id: `playlist:${op.id}:tracks`,
              kind: "playlistTracks",
              name: playlist.name,
              local: String(patch.tracks.length),
              remote: String(playlist.tracks.length),
            });
          } else if (changedLocally) {
            playlist.tracks = [...patch.tracks];
            markStale(playlist);
          }
        }

        const hasCoverFile = patch.coverFile !== undefined;
        const coverTouched = patch.cover !== undefined || hasCoverFile || patch.clearCover === true;
        if (coverTouched) {
          const localCover = hasCoverFile ? patch.coverFile : (patch.clearCover ? undefined : patch.cover);
          const remoteCover = playlist.coverFile ?? playlist.cover;
          if (localCover !== remoteCover) {
            const baseCover = basePlaylist?.coverFile ?? basePlaylist?.cover;
            const changedLocally = localChanged(localCover, baseCover);
            const changedRemotely = remoteChanged(remoteCover, baseCover);
            if (changedLocally && changedRemotely) {
              pushConflict(conflicts, {
                id: `playlist:${op.id}:cover`,
                kind: "playlistCover",
                name: playlist.name,
              });
            } else if (changedLocally) {
              playlist.cover = patch.clearCover ? undefined : patch.cover;
              playlist.coverFile = patch.coverFile;
            }
          }
        }

        const coverTrackTouched = patch.coverTrack !== undefined || patch.clearCoverTrack === true;
        if (coverTrackTouched) {
          const localCoverTrack = patch.clearCoverTrack ? undefined : patch.coverTrack;
          if (playlist.coverTrack !== localCoverTrack) {
            const baseCoverTrack = basePlaylist?.coverTrack;
            const changedLocally = localChanged(localCoverTrack, baseCoverTrack);
            const changedRemotely = remoteChanged(playlist.coverTrack, baseCoverTrack);
            if (changedLocally && changedRemotely) {
              pushConflict(conflicts, {
                id: `playlist:${op.id}:cover`,
                kind: "playlistCover",
                name: playlist.name,
              });
            } else if (changedLocally) {
              playlist.coverTrack = localCoverTrack;
            }
          }
        }

        const smartTouched = patch.smartTransition !== undefined || patch.clearSmartTransition === true;
        if (smartTouched) {
          const localSmart = patch.clearSmartTransition ? undefined : patch.smartTransition;
          if (!sameValue(playlist.smartTransition, localSmart)) {
            const baseSmart = basePlaylist?.smartTransition;
            const changedLocally = localChanged(localSmart, baseSmart);
            const changedRemotely = remoteChanged(playlist.smartTransition, baseSmart);
            if (changedLocally && changedRemotely) {
              pushConflict(conflicts, {
                id: `playlist:${op.id}:smart`,
                kind: "playlistTracks",
                name: playlist.name,
              });
            } else if (changedLocally) {
              playlist.smartTransition = localSmart;
            }
          }
        }
        break;
      }
      case "markSmartTransitionStale": {
        const playlist = findPlaylist(profile, op.id);
        if (playlist) markStale(playlist);
        break;
      }
    }
  }

  return { profile, conflicts };
}

function markStale(playlist: Playlist): void {
  if (!playlist.smartTransition || playlist.smartTransition.status !== "ready") return;
  playlist.smartTransition = { ...playlist.smartTransition, status: "stale" };
}

/** Restore the pending operations and their base snapshot from local storage. */
export function loadProfileSyncState(): PersistedProfileSync | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PersistedProfileSync>;
    if (!parsed || !Array.isArray(parsed.ops) || !parsed.ops.length) return null;
    if (!parsed.baseProfile) return null;
    return {
      ops: parsed.ops,
      baseRevision: typeof parsed.baseRevision === "string" ? parsed.baseRevision : null,
      baseProfile: parsed.baseProfile as Profile,
    };
  } catch {
    return null;
  }
}

/** Persist pending edits so an offline session can still sync after a restart. */
export function saveProfileSyncState(state: PersistedProfileSync): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Quota or private mode: the in-memory queue still works this session.
  }
}

/** Drop the persisted queue once everything has reached the cloud. */
export function clearProfileSyncState(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Best effort only.
  }
}
