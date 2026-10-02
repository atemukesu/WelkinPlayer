import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import {
  createDefaultProfile,
  createId,
  parseProfile,
} from "../lib/profile";
import type { Playlist, Profile } from "../lib/profile";
import { coverUrl } from "../lib/remote";
import { profileTrackKey, sourceProfileId, splitTrackKey, trackKey } from "../lib/sources";
import { useSourcesStore } from "./sources";
import type { Track } from "./player";

/** Where the loaded profile came from. */
export type ProfileSource = "remote" | "local" | "none";

/** Outcome of probing the server for an existing profile document. */
export type RemoteProbe = "found" | "missing" | "error";

interface LoadedProfile {
  content: string | null;
  source: ProfileSource;
  remoteError: string | null;
  remoteMissing: boolean;
}

interface SaveProfileResult {
  remote: boolean;
  local: boolean;
  warning: string | null;
}

/** Mirrors the Rust `UploadedCover` struct returned by `upload_playlist_cover`. */
interface UploadedCover {
  file: string;
  hash: string;
  path: string | null;
}

/** Mirrors the Rust `ResolvedCover` struct returned by `resolve_playlist_covers`. */
interface ResolvedCover {
  file: string;
  path: string | null;
}

const SAVE_DEBOUNCE_MS = 900;
const RECENT_LIMIT = 12;

export const useProfileStore = defineStore("profile", () => {
  const profile = ref<Profile>(createDefaultProfile());
  const ready = ref(false);
  const firstRun = ref(false);
  const source = ref<ProfileSource>("none");
  const remoteError = ref<string | null>(null);
  const saving = ref(false);
  const lastSavedAt = ref<number | null>(null);
  const lastSaveWarning = ref<string | null>(null);
  /** Bumped whenever the profile is (re)applied, so views can re-sync. */
  const revision = ref(0);
  /** Resolved local asset URLs for uploaded covers, keyed by remote file name. */
  const coverUrls = ref<Record<string, string>>({});

  let saveTimer = 0;
  let pending = false;
  let hadLocalCopy = false;

  function profileKey(key: string): string {
    const { sourceId, path } = splitTrackKey(key);
    const sourceStore = useSourcesStore();
    const source = sourceStore.sources.find((item) => item.id === sourceId);
    if (source) return `${sourceProfileId(source)}::${path}`;
    // Older profiles stored a random source id. When there is only one cloud
    // source configured, it is safe to associate those legacy keys with it.
    const cloudSources = sourceStore.sources.filter((item) => item.kind === "webdav");
    return cloudSources.length === 1 ? `${sourceProfileId(cloudSources[0])}::${path}` : key;
  }

  function migrateKeys(value: Profile): { value: Profile; changed: boolean } {
    let changed = false;
    const migrate = (key: string) => {
      const next = profileKey(key);
      changed ||= next !== key;
      return next;
    };
    value.favorites = value.favorites.map(migrate);
    value.disabledLyrics = value.disabledLyrics.map(migrate);
    value.recent = value.recent.map(migrate);
    const playCounts: Record<string, number> = {};
    for (const [key, count] of Object.entries(value.playCounts)) {
      const next = migrate(key);
      playCounts[next] = (playCounts[next] ?? 0) + count;
    }
    value.playCounts = playCounts;
    value.playlists = value.playlists.map((playlist) => ({ ...playlist, tracks: playlist.tracks.map(migrate), coverTrack: playlist.coverTrack ? migrate(playlist.coverTrack) : playlist.coverTrack }));
    return { value, changed };
  }

  const nickname = computed(() => profile.value.nickname.trim());
  const playlists = computed(() => profile.value.playlists);
  const favorites = computed(() => profile.value.favorites);
  const disabledLyrics = computed(() => profile.value.disabledLyrics);
  const recent = computed(() => profile.value.recent);
  const hasRemoteCopy = computed(() => source.value === "remote");

  function touch() {
    profile.value.updatedAt = Date.now();
  }

  /**
   * Load the local cache immediately (no network). Returns `true` when the
   * first-run wizard should show. The authoritative remote copy is fetched
   * separately: [`syncRemote`] for returning users, or [`fetchRemote`] from the
   * wizard once server credentials are known.
   */
  async function hydrate(): Promise<boolean> {
    if (ready.value) return firstRun.value;
    const fallback = createDefaultProfile();
    try {
      const local = await invoke<LoadedProfile>("load_local_profile");
      hadLocalCopy = local.content !== null;
      source.value = local.source;
      const migrated = migrateKeys(parseProfile(local.content, fallback));
      profile.value = migrated.value;
      if (migrated.changed) scheduleSave();
      firstRun.value = local.content === null || !profile.value.initialized;
      revision.value += 1;
    } catch (error) {
      remoteError.value = error instanceof Error ? error.message : String(error);
      profile.value = fallback;
      firstRun.value = true;
    } finally {
      ready.value = true;
    }
    void resolveCovers();
    return firstRun.value;
  }

  /** Fetch the authoritative remote copy and replace the cache when available. */
  async function syncRemote(): Promise<void> {
    try {
      const remote = await invoke<LoadedProfile>("load_remote_profile");
      remoteError.value = remote.remoteError;
      if (remote.source === "remote" && remote.content !== null) {
        const migrated = migrateKeys(parseProfile(remote.content, profile.value));
        profile.value = migrated.value;
        if (migrated.changed) scheduleSave();
        source.value = "remote";
        firstRun.value = !profile.value.initialized;
        revision.value += 1;
        void resolveCovers();
        void migrateLegacyCovers();
        return;
      }
      // Server reachable but has no document: seed it with the local cache so
      // the cloud becomes authoritative from here on.
      if (remote.remoteMissing && hadLocalCopy) {
        void migrateLegacyCovers();
        scheduleSave();
        return;
      }
      source.value = remote.source;
    } catch (error) {
      remoteError.value = error instanceof Error ? error.message : String(error);
    }
  }

  /**
   * Probe the server for an existing profile without any seeding side effects.
   *
   * Used by the first-run wizard *after* the user has saved WebDAV credentials:
   * when an initialized document is found it is applied and the wizard skips
   * the remaining configuration instead of overwriting the remote copy.
   */
  async function fetchRemote(): Promise<RemoteProbe> {
    try {
      const remote = await invoke<LoadedProfile>("load_remote_profile");
      remoteError.value = remote.remoteError;
      if (remote.source === "remote" && remote.content !== null) {
        const migrated = migrateKeys(parseProfile(remote.content, profile.value));
        profile.value = migrated.value;
        if (migrated.changed) scheduleSave();
        source.value = "remote";
        revision.value += 1;
        void resolveCovers();
        void migrateLegacyCovers();
        return profile.value.initialized ? "found" : "missing";
      }
      source.value = remote.source;
      return remote.remoteError ? "error" : "missing";
    } catch (error) {
      remoteError.value = error instanceof Error ? error.message : String(error);
      return "error";
    }
  }

  async function flush(): Promise<void> {
    window.clearTimeout(saveTimer);
    if (pending) {
      pending = false;
      saving.value = true;
      try {
        const result = await invoke<SaveProfileResult>("save_profile", {
          content: JSON.stringify(profile.value),
        });
        lastSaveWarning.value = result.warning;
        if (result.remote) source.value = "remote";
        else if (result.warning) source.value = "local";
        lastSavedAt.value = Date.now();
      } catch (error) {
        lastSaveWarning.value = error instanceof Error ? error.message : String(error);
      } finally {
        saving.value = false;
      }
    }
  }

  /** Schedule a debounced persist, coalescing bursts of edits. */
  function scheduleSave() {
    pending = true;
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => void flush(), SAVE_DEBOUNCE_MS);
  }

  function setNickname(value: string) {
    profile.value.nickname = value;
    touch();
    scheduleSave();
  }

  function completeSetup() {
    profile.value.initialized = true;
    firstRun.value = false;
    touch();
    return flush();
  }

  /**
   * Close the wizard after an existing remote profile was loaded, without
   * writing anything back: the remote document is already authoritative.
   */
  function finishExisting() {
    firstRun.value = false;
  }

  function recordPlay(path: string | undefined) {
    if (!path) return;
    path = profileKey(path);
    profile.value.playCounts[path] = (profile.value.playCounts[path] ?? 0) + 1;
    profile.value.recent = [path, ...profile.value.recent.filter((item) => item !== path)].slice(0, RECENT_LIMIT);
    scheduleSave();
  }

  /**
   * Drop the legacy playback fields from the profile. They now live in the
   * dedicated playback store; this runs once after the migration seed.
   */
  function clearLegacyPlayback() {
    if (profile.value.lastTrack === undefined && profile.value.lastPosition === undefined) return;
    delete profile.value.lastTrack;
    delete profile.value.lastPosition;
    touch();
    scheduleSave();
  }

  function isFavorite(path: string | undefined): boolean {
    return !!path && profile.value.favorites.includes(profileKey(path));
  }

  function toggleFavorite(path: string | undefined) {
    if (!path) return;
    path = profileKey(path);
    const index = profile.value.favorites.indexOf(path);
    if (index >= 0) profile.value.favorites.splice(index, 1);
    else profile.value.favorites.push(path);
    scheduleSave();
  }

  /** Whether the track's lyrics are suppressed (per-track opt-out). */
  function isLyricsDisabled(path: string | undefined): boolean {
    return !!path && profile.value.disabledLyrics.includes(profileKey(path));
  }

  /** Suppress or restore lyrics for one track. */
  function setLyricsDisabled(path: string | undefined, disabled: boolean) {
    if (!path) return;
    path = profileKey(path);
    const has = profile.value.disabledLyrics.includes(path);
    if (disabled === has) return;
    if (disabled) profile.value.disabledLyrics.push(path);
    else profile.value.disabledLyrics = profile.value.disabledLyrics.filter((item) => item !== path);
    scheduleSave();
  }

  function toggleLyricsDisabled(path: string | undefined) {
    if (!path) return;
    setLyricsDisabled(path, !isLyricsDisabled(path));
  }

  function createPlaylist(name: string): Playlist {
    const playlist: Playlist = { id: createId(), name: name.trim() || "Playlist", tracks: [] };
    profile.value.playlists.push(playlist);
    scheduleSave();
    return playlist;
  }

  function renamePlaylist(id: string, name: string) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    playlist.name = name.trim() || playlist.name;
    scheduleSave();
  }

  /** Update mutable playlist fields (name, cover and/or tracks). */
  function updatePlaylist(id: string, patch: Partial<Pick<Playlist, "name" | "cover" | "coverFile" | "coverTrack" | "tracks">>) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    if (patch.name !== undefined) playlist.name = patch.name.trim() || playlist.name;
    if (patch.tracks !== undefined) playlist.tracks = [...patch.tracks];
    if ("cover" in patch) playlist.cover = patch.cover;
    if ("coverFile" in patch) playlist.coverFile = patch.coverFile;
    if ("coverTrack" in patch) playlist.coverTrack = patch.coverTrack;
    scheduleSave();
  }

  /** Unique cover files currently referenced by any playlist. */
  function referencedCoverFiles(): string[] {
    const files = new Set<string>();
    for (const playlist of profile.value.playlists) {
      if (playlist.coverFile) files.add(playlist.coverFile);
    }
    return [...files];
  }

  /** Resolve referenced cover files to cached asset URLs (cache-first). */
  async function resolveCovers(): Promise<void> {
    const files = referencedCoverFiles();
    // Keep only still-referenced entries so removed covers release their URLs.
    const kept: Record<string, string> = {};
    for (const file of files) {
      const existing = coverUrls.value[file];
      if (existing) kept[file] = existing;
    }
    coverUrls.value = kept;
    if (files.length === 0) return;
    try {
      const resolved = await invoke<ResolvedCover[]>("resolve_playlist_covers", { files });
      const merged = { ...coverUrls.value };
      for (const item of resolved) {
        const url = coverUrl(item.path);
        if (url) merged[item.file] = url;
      }
      coverUrls.value = merged;
    } catch (error) {
      console.warn("[welkin] failed to resolve playlist covers", error);
    }
  }

  /** Upload an image data URL as a standalone cover file and cache it. */
  async function uploadCover(dataUrl: string): Promise<string> {
    const result = await invoke<UploadedCover>("upload_playlist_cover", { dataUrl });
    const url = coverUrl(result.path, result.hash);
    if (url) coverUrls.value = { ...coverUrls.value, [result.file]: url };
    return result.file;
  }

  /** Attach an uploaded cover to a playlist, cleaning up a replaced one. */
  async function setPlaylistCover(id: string, dataUrl: string): Promise<void> {
    const file = await uploadCover(dataUrl);
    const previous = profile.value.playlists.find((item) => item.id === id)?.coverFile;
    updatePlaylist(id, { cover: undefined, coverFile: file });
    if (previous && previous !== file) void pruneCover(previous);
  }

  /** Remove a playlist's uploaded cover and delete it when now unreferenced. */
  function clearPlaylistCover(id: string): void {
    const file = profile.value.playlists.find((item) => item.id === id)?.coverFile;
    updatePlaylist(id, { cover: undefined, coverFile: undefined });
    if (file) void pruneCover(file);
  }

  /** Delete a cover object unless another playlist still shares its content. */
  async function pruneCover(file: string): Promise<void> {
    if (profile.value.playlists.some((item) => item.coverFile === file)) return;
    const next = { ...coverUrls.value };
    delete next[file];
    coverUrls.value = next;
    try {
      await invoke("delete_playlist_cover", { file });
    } catch (error) {
      console.warn("[welkin] failed to delete playlist cover", error);
    }
  }

  /** Local asset URL for a playlist cover, or `undefined` to fall back. */
  function playlistCoverUrl(playlist: Playlist): string | undefined {
    if (playlist.cover) return playlist.cover;
    if (playlist.coverFile) return coverUrls.value[playlist.coverFile];
    return undefined;
  }

  /** Re-upload legacy data-URL covers as standalone files (best effort). */
  async function migrateLegacyCovers(): Promise<void> {
    const legacy = profile.value.playlists.filter((item) => item.cover?.startsWith("data:"));
    if (legacy.length === 0) return;
    for (const playlist of legacy) {
      try {
        const file = await uploadCover(playlist.cover as string);
        const target = profile.value.playlists.find((item) => item.id === playlist.id);
        if (!target) continue;
        target.cover = undefined;
        target.coverFile = file;
      } catch (error) {
        console.warn("[welkin] failed to migrate playlist cover", error);
      }
    }
    touch();
    scheduleSave();
  }

  function deletePlaylist(id: string) {
    const file = profile.value.playlists.find((item) => item.id === id)?.coverFile;
    profile.value.playlists = profile.value.playlists.filter((item) => item.id !== id);
    scheduleSave();
    if (file) void pruneCover(file);
  }

  function addToPlaylist(id: string, path: string | undefined) {
    if (!path) return;
    path = profileKey(path);
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist || playlist.tracks.includes(path)) return;
    playlist.tracks.push(path);
    scheduleSave();
  }

  function removeFromPlaylist(id: string, path: string) {
    path = profileKey(path);
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    playlist.tracks = playlist.tracks.filter((item) => item !== path);
    scheduleSave();
  }

  function moveTrackInPlaylist(id: string, path: string, direction: "up" | "down") {
    path = profileKey(path);
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    const index = playlist.tracks.indexOf(path);
    if (index < 0) return;
    const target = direction === "up" ? index - 1 : index + 1;
    if (target < 0 || target >= playlist.tracks.length) return;
    const next = [...playlist.tracks];
    [next[index], next[target]] = [next[target], next[index]];
    playlist.tracks = next;
    scheduleSave();
  }

  function getTrackIndexInPlaylist(id: string, path: string): { index: number; total: number } {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return { index: -1, total: 0 };
    return { index: playlist.tracks.indexOf(path), total: playlist.tracks.length };
  }

  return {
    profile,
    ready,
    firstRun,
    source,
    remoteError,
    saving,
    lastSavedAt,
    lastSaveWarning,
    revision,
    coverUrls,
    nickname,
    playlists,
    favorites,
    disabledLyrics,
    recent,
    hasRemoteCopy,
    hydrate,
    syncRemote,
    fetchRemote,
    flush,
    scheduleSave,
    setNickname,
    completeSetup,
    finishExisting,
    recordPlay,
    clearLegacyPlayback,
    isFavorite,
    toggleFavorite,
    isLyricsDisabled,
    setLyricsDisabled,
    toggleLyricsDisabled,
    createPlaylist,
    renamePlaylist,
    updatePlaylist,
    deletePlaylist,
    addToPlaylist,
    removeFromPlaylist,
    moveTrackInPlaylist,
    getTrackIndexInPlaylist,
    resolveCovers,
    uploadCover,
    setPlaylistCover,
    clearPlaylistCover,
    pruneCover,
    playlistCoverUrl,
    migrateLegacyCovers,
  };
});

/**
 * Resolve profile keys to library tracks, keeping the keys' order. Keys with no
 * local match become greyed-out placeholder tracks (see {@link placeholderTrack})
 * instead of being dropped, so a playlist synced from another device shows the
 * tracks this device is missing.
 */
export function tracksForPaths(keys: string[], tracks: Track[]): Track[] {
  const byKey = new Map<string, Track>();
  for (const track of tracks) {
    if (!track.path) continue;
    byKey.set(trackKey(track) as string, track);
    const profileKey = profileKeyForTrack(track);
    if (profileKey) byKey.set(profileKey, track);
  }
  return keys.map((key) => byKey.get(key) ?? placeholderTrack(key));
}

/** Stable non-positive id derived from a profile key, so placeholders never collide with real (positive) track ids. */
function placeholderId(key: string): number {
  let hash = 2166136261;
  for (let index = 0; index < key.length; index += 1) {
    hash ^= key.charCodeAt(index);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return -(hash % 2147483647) - 1;
}

/**
 * Build a display-only track for a profile key this device cannot resolve. It
 * has no `path`/`sourceId`, so it is never queued, streamed or selection-matched;
 * the UI renders it greyed out and clicking it only shows a hint.
 */
export function placeholderTrack(key: string): Track {
  const { sourceId, path } = splitTrackKey(key);
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  const title = name.replace(/\.[^.]+$/, "") || name || key;
  const source = useSourcesStore().sources.find((item) => item.id === sourceId || sourceProfileId(item) === sourceId);
  return {
    id: placeholderId(key),
    title,
    artist: source?.name ?? "",
    album: path,
    duration: "--:--",
    color: "#6b7280",
    profileKey: key,
    unavailable: true,
  };
}

/**
 * Normalized key a track is stored under in the profile (favorites, play counts,
 * recent, playlists). WebDAV sources map to a stable `dav-<hash>` namespace so
 * the data survives across devices; local sources keep their machine-local UUID.
 * Use this for any play-count/favorite lookup instead of the device-local
 * `trackKey`.
 */
export function profileKeyForTrack(
  track: { sourceId?: string; path?: string } | null | undefined,
): string | undefined {
  if (!track?.path) return undefined;
  const source = useSourcesStore().sources.find((item) => item.id === track.sourceId);
  return profileTrackKey({ sourceId: track.sourceId, path: track.path, profileSourceId: source ? sourceProfileId(source) : undefined });
}
