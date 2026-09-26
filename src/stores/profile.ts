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
      profile.value = parseProfile(local.content, fallback);
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
        profile.value = parseProfile(remote.content, profile.value);
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
        profile.value = parseProfile(remote.content, profile.value);
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
    return !!path && profile.value.favorites.includes(path);
  }

  function toggleFavorite(path: string | undefined) {
    if (!path) return;
    const index = profile.value.favorites.indexOf(path);
    if (index >= 0) profile.value.favorites.splice(index, 1);
    else profile.value.favorites.push(path);
    scheduleSave();
  }

  /** Whether the track's lyrics are suppressed (per-track opt-out). */
  function isLyricsDisabled(path: string | undefined): boolean {
    return !!path && profile.value.disabledLyrics.includes(path);
  }

  /** Suppress or restore lyrics for one track. */
  function setLyricsDisabled(path: string | undefined, disabled: boolean) {
    if (!path) return;
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
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist || playlist.tracks.includes(path)) return;
    playlist.tracks.push(path);
    scheduleSave();
  }

  function removeFromPlaylist(id: string, path: string) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    playlist.tracks = playlist.tracks.filter((item) => item !== path);
    scheduleSave();
  }

  function moveTrackInPlaylist(id: string, path: string, direction: "up" | "down") {
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

/** Keep a track lookup helper close to the playlist consumers. */
export function tracksForPaths(paths: string[], tracks: Track[]): Track[] {
  const byPath = new Map(tracks.filter((track) => track.path).map((track) => [track.path as string, track]));
  return paths.flatMap((path) => {
    const track = byPath.get(path);
    return track ? [track] : [];
  });
}
