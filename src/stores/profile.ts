import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import {
  createDefaultProfile,
  createId,
  parseProfile,
} from "../lib/profile";
import type { Playlist, Profile } from "../lib/profile";
import type { Track } from "./player";

/** Where the loaded profile came from. */
export type ProfileSource = "remote" | "local" | "none";

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

  let saveTimer = 0;
  let pending = false;
  let hadLocalCopy = false;

  const nickname = computed(() => profile.value.nickname.trim());
  const playlists = computed(() => profile.value.playlists);
  const favorites = computed(() => profile.value.favorites);
  const recent = computed(() => profile.value.recent);
  const hasRemoteCopy = computed(() => source.value === "remote");

  function touch() {
    profile.value.updatedAt = Date.now();
  }

  /**
   * Load the local cache immediately (no network), then fetch the
   * authoritative remote copy in the background. Returns `true` when the
   * first-run wizard should show.
   */
  async function hydrate(seed?: Partial<Profile["appearance"] & Profile["lyrics"]>): Promise<boolean> {
    if (ready.value) return firstRun.value;
    const fallback = createDefaultProfile(seed);
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
    void syncRemote();
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
        return;
      }
      // Server reachable but has no document: seed it with the local cache so
      // the cloud becomes authoritative from here on.
      if (remote.remoteMissing && hadLocalCopy) {
        scheduleSave();
        return;
      }
      source.value = remote.source;
    } catch (error) {
      remoteError.value = error instanceof Error ? error.message : String(error);
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

  function setAppearance(patch: Partial<Profile["appearance"]>) {
    profile.value.appearance = { ...profile.value.appearance, ...patch };
    touch();
    scheduleSave();
  }

  function setLyrics(patch: Partial<Profile["lyrics"]>) {
    profile.value.lyrics = { ...profile.value.lyrics, ...patch };
    touch();
    scheduleSave();
  }

  function completeSetup() {
    profile.value.initialized = true;
    firstRun.value = false;
    touch();
    return flush();
  }

  function recordPlay(path: string | undefined) {
    if (!path) return;
    profile.value.playCounts[path] = (profile.value.playCounts[path] ?? 0) + 1;
    profile.value.recent = [path, ...profile.value.recent.filter((item) => item !== path)].slice(0, RECENT_LIMIT);
    scheduleSave();
  }

  /** Remember the last playing track and its position, for resume. */
  function recordPlayback(path: string | undefined, position: number) {
    if (!path) return;
    profile.value.lastTrack = path;
    profile.value.lastPosition = Math.max(0, Math.round(position));
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

  /** Update mutable playlist fields (name and/or cover). */
  function updatePlaylist(id: string, patch: Partial<Pick<Playlist, "name" | "cover" | "coverTrack">>) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    if (patch.name !== undefined) playlist.name = patch.name.trim() || playlist.name;
    if ("cover" in patch) playlist.cover = patch.cover;
    if ("coverTrack" in patch) playlist.coverTrack = patch.coverTrack;
    scheduleSave();
  }

  function deletePlaylist(id: string) {
    profile.value.playlists = profile.value.playlists.filter((item) => item.id !== id);
    scheduleSave();
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
    nickname,
    playlists,
    favorites,
    recent,
    hasRemoteCopy,
    hydrate,
    syncRemote,
    flush,
    scheduleSave,
    setNickname,
    setAppearance,
    setLyrics,
    completeSetup,
    recordPlay,
    recordPlayback,
    isFavorite,
    toggleFavorite,
    createPlaylist,
    renamePlaylist,
    updatePlaylist,
    deletePlaylist,
    addToPlaylist,
    removeFromPlaylist,
    moveTrackInPlaylist,
    getTrackIndexInPlaylist,
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
