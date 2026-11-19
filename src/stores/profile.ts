/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import {
  createDefaultProfile,
  createId,
  parseProfile,
} from "../lib/profile";
import type { Playlist, Profile } from "../lib/profile";
import {
  clearProfileSyncState,
  cloneProfile,
  loadProfileSyncState,
  replayProfileOps,
  saveProfileSyncState,
} from "../lib/profileSync";
import type { PlaylistPatch, ProfileConflict, ProfileOp } from "../lib/profileSync";
import { coverUrl } from "../lib/remote";
import { deleteSmartTransition } from "../lib/smartTransition";
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
  revision: string | null;
}

interface SaveProfileResult {
  remote: boolean;
  local: boolean;
  warning: string | null;
  conflict: boolean;
  revision: string | null;
}

/** A conflict awaiting the user's choice in the sync dialog. */
export interface ProfileConflictSession {
  conflicts: ProfileConflict[];
  local: Profile;
  remote: Profile;
  remoteRevision: string | null;
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
  /** Pending cloud conflict; non-null while the user must pick a version. */
  const conflict = ref<ProfileConflictSession | null>(null);
  /** Resolved local asset URLs for uploaded covers, keyed by remote file name. */
  const coverUrls = ref<Record<string, string>>({});

  let saveTimer = 0;
  let pending = false;
  /** Snapshot of the last cloud profile the queued operations were based on. */
  let baseProfile: Profile = createDefaultProfile();
  let baseRevision: string | null = null;
  /** Local edits that have not reached the cloud yet, in order. */
  let outbox: ProfileOp[] = [];
  let running = false;
  let rerun = false;
  let retryTimer = 0;
  /** Cover objects to delete once a merge has safely reached the cloud. */
  const pendingCoverPrunes = new Set<string>();

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
      source.value = local.source;
      const migrated = migrateKeys(parseProfile(local.content, fallback));
      profile.value = migrated.value;
      // Resume an offline queue from a previous session; otherwise the local
      // profile becomes the baseline for the first cloud merge.
      const restored = loadProfileSyncState();
      if (restored) {
        outbox = restored.ops;
        baseRevision = restored.baseRevision;
        baseProfile = restored.baseProfile;
        // The local cache may have been overwritten by the last cloud read;
        // rebuilding from the baseline + queue restores this device's view.
        profile.value = replayProfileOps(baseProfile, baseProfile, outbox).profile;
        pending = true;
        scheduleSave();
      } else {
        baseProfile = cloneProfile(profile.value);
        baseRevision = null;
        if (migrated.changed) scheduleSave();
      }
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

  /** Persist the queue + baseline so a restart can resume pending edits. */
  function persistSyncState() {
    if (outbox.length > 0) saveProfileSyncState({ ops: outbox, baseRevision, baseProfile });
    else clearProfileSyncState();
  }

  function enqueue(op: ProfileOp) {
    outbox.push(op);
    persistSyncState();
    scheduleSave();
  }

  function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  async function saveContent(
    content: Profile,
    expectedRevision: string | null,
    force: boolean,
  ): Promise<SaveProfileResult> {
    try {
      return await invoke<SaveProfileResult>("save_profile", {
        content: JSON.stringify(content),
        expectedRevision,
        force,
      });
    } catch (error) {
      return {
        remote: false,
        local: false,
        warning: errorMessage(error),
        conflict: false,
        revision: null,
      };
    }
  }

  /** Adopt an unmodified cloud document, discarding nothing local. */
  function adoptRemote(remote: Profile, remoteRevision: string | null) {
    profile.value = cloneProfile(remote);
    baseProfile = cloneProfile(remote);
    baseRevision = remoteRevision;
    outbox = [];
    pending = false;
    source.value = "remote";
    firstRun.value = !profile.value.initialized;
    lastSavedAt.value = Date.now();
    revision.value += 1;
    clearProfileSyncState();
    void resolveCovers();
    void migrateLegacyCovers();
    void runCoverPrunes();
  }

  /** The local queue is now part of the cloud document. */
  function onSaved(merged: Profile, revisionToken: string | null, applied: ProfileOp[]) {
    outbox = outbox.slice(applied.length);
    const remaining = [...outbox];
    let working = merged;
    if (remaining.length > 0) {
      working = replayProfileOps(merged, working, remaining).profile;
    }
    profile.value = working;
    baseProfile = cloneProfile(merged);
    baseRevision = revisionToken;
    pending = outbox.length > 0;
    source.value = "remote";
    lastSavedAt.value = Date.now();
    lastSaveWarning.value = null;
    revision.value += 1;
    if (pending) {
      persistSyncState();
      scheduleSave();
    } else {
      clearProfileSyncState();
    }
    void resolveCovers();
    void migrateLegacyCovers();
    void runCoverPrunes();
  }

  function raiseConflict(conflicts: ProfileConflict[], remote: Profile, remoteRevision: string | null) {
    conflict.value = {
      conflicts,
      local: cloneProfile(profile.value),
      remote,
      remoteRevision,
    };
  }

  /**
   * One read-merge-write pass.
   *
   * - Pulls the authoritative cloud copy first.
   * - Replays locally queued operations on top of it.
   * - Skips the write entirely when the cloud already contains everything.
   * - Retries once on a stale revision (HTTP 412), then asks the user.
   */
  async function syncOnce(): Promise<"done" | "retry" | "conflict"> {
    if (conflict.value) return "conflict";
    let lastRemote: Profile | null = null;
    let lastRevision: string | null = null;

    for (let attempt = 0; attempt < 2; attempt += 1) {
      let loaded: LoadedProfile;
      try {
        loaded = await invoke<LoadedProfile>("load_remote_profile");
      } catch (error) {
        lastSaveWarning.value = errorMessage(error);
        return "retry";
      }
      remoteError.value = loaded.remoteError;
      if (loaded.remoteError) {
        // Do not overwrite a cloud document we could not read.
        lastSaveWarning.value = loaded.remoteError;
        return "retry";
      }

      if (loaded.source !== "remote") {
        // Either no sync source is configured or the server has no document.
        // Without a remote and without pending edits there is nothing to push.
        if (outbox.length === 0 && !loaded.remoteMissing) {
          lastSaveWarning.value = null;
          return "done";
        }
        const ops = [...outbox];
        const local = cloneProfile(profile.value);
        const result = await saveContent(local, null, false);
        if (result.conflict) continue;
        const noRemote = !loaded.remoteMissing;
        if (noRemote || result.remote) {
          outbox = outbox.slice(ops.length);
          baseProfile = cloneProfile(local);
          baseRevision = result.revision ?? null;
          pending = outbox.length > 0;
          source.value = result.remote ? "remote" : "local";
          lastSavedAt.value = Date.now();
          lastSaveWarning.value = result.warning;
          if (pending) {
            persistSyncState();
            scheduleSave();
          } else {
            clearProfileSyncState();
            void runCoverPrunes();
          }
          return "done";
        }
        lastSaveWarning.value = result.warning;
        return "retry";
      }

      if (loaded.content === null || loaded.revision === null) return "retry";
      const migrated = migrateKeys(parseProfile(loaded.content, createDefaultProfile()));
      const remoteProfile = migrated.value;
      lastRemote = remoteProfile;
      lastRevision = loaded.revision;

      const ops = [...outbox];
      const replayed = ops.length > 0
        ? replayProfileOps(baseProfile, remoteProfile, ops)
        : { profile: remoteProfile, conflicts: [] as ProfileConflict[] };
      if (replayed.conflicts.length > 0) {
        raiseConflict(replayed.conflicts, remoteProfile, loaded.revision);
        return "conflict";
      }

      const merged = replayed.profile;
      const needWrite = ops.length > 0 || migrated.changed;
      if (!needWrite) {
        if (outbox.length === 0) {
          adoptRemote(remoteProfile, loaded.revision);
          return "done";
        }
        // New edits arrived while the read was in flight; replay them too.
        rerun = true;
        continue;
      }

      merged.updatedAt = Date.now();
      const saved = await saveContent(merged, loaded.revision, false);
      if (saved.conflict) continue;
      if (!saved.remote) {
        lastSaveWarning.value = saved.warning;
        return "retry";
      }
      onSaved(merged, saved.revision ?? loaded.revision, ops);
      return "done";
    }

    // Two consecutive 412s: another device is writing continuously. Ask.
    if (lastRemote) {
      raiseConflict(
        [{ id: "remote", kind: "remoteChanged", name: "" }],
        lastRemote,
        lastRevision,
      );
      return "conflict";
    }
    return "retry";
  }

  /**
   * Serialized synchronization loop. Every queued edit goes through this, and
   * an explicit call (startup / focus) also pulls remote-only changes.
   */
  async function syncNow(): Promise<void> {
    window.clearTimeout(saveTimer);
    window.clearTimeout(retryTimer);
    if (conflict.value) return;
    if (running) {
      rerun = true;
      return;
    }
    running = true;
    saving.value = true;
    let status: "done" | "retry" | "conflict" = "done";
    try {
      do {
        rerun = false;
        status = await syncOnce();
        if (status === "conflict") break;
      } while (status === "done" && (rerun || outbox.length > 0));
    } finally {
      running = false;
      saving.value = false;
    }
    if (status === "retry") {
      retryTimer = window.setTimeout(() => void syncNow(), 30_000);
    } else if (status === "done" && pending) {
      scheduleSave();
    }
  }

  /** Fetch the authoritative remote copy (merging any pending local edits). */
  async function syncRemote(): Promise<void> {
    await syncNow();
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
        baseProfile = cloneProfile(profile.value);
        baseRevision = remote.revision;
        outbox = [];
        pending = false;
        clearProfileSyncState();
        source.value = "remote";
        revision.value += 1;
        void resolveCovers();
        void migrateLegacyCovers();
        if (migrated.changed) scheduleSave();
        return profile.value.initialized ? "found" : "missing";
      }
      source.value = remote.source;
      baseProfile = cloneProfile(profile.value);
      baseRevision = null;
      return remote.remoteError ? "error" : "missing";
    } catch (error) {
      remoteError.value = errorMessage(error);
      return "error";
    }
  }

  async function flush(): Promise<void> {
    await syncNow();
  }

  /** Schedule a debounced sync, coalescing bursts of edits. */
  function scheduleSave() {
    pending = true;
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => void syncNow(), SAVE_DEBOUNCE_MS);
  }

  /** Resolve a pending conflict with the version the user chose. */
  async function resolveConflict(choice: "local" | "remote"): Promise<void> {
    const session = conflict.value;
    if (!session) return;
    if (choice === "remote") {
      adoptRemote(session.remote, session.remoteRevision);
      conflict.value = null;
      return;
    }
    // Keep the local version: force a write that ignores the cloud revision.
    saving.value = true;
    try {
      touch();
      const result = await saveContent(profile.value, null, true);
      if (!result.local) {
        lastSaveWarning.value = result.warning;
        return;
      }
      outbox = [];
      pending = false;
      baseProfile = cloneProfile(profile.value);
      baseRevision = result.revision ?? null;
      source.value = result.remote ? "remote" : "local";
      lastSavedAt.value = Date.now();
      lastSaveWarning.value = result.warning;
      clearProfileSyncState();
      conflict.value = null;
      revision.value += 1;
      void resolveCovers();
      void runCoverPrunes();
    } finally {
      saving.value = false;
    }
  }

  function setNickname(value: string) {
    profile.value.nickname = value;
    touch();
    enqueue({ type: "setNickname", value });
    scheduleSave();
  }

  function completeSetup() {
    profile.value.initialized = true;
    firstRun.value = false;
    touch();
    enqueue({ type: "setInitialized", value: true });
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
    enqueue({ type: "recordPlay", key: path });
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
    enqueue({ type: "clearLegacyPlayback" });
    scheduleSave();
  }

  function isFavorite(path: string | undefined): boolean {
    return !!path && profile.value.favorites.includes(profileKey(path));
  }

  function toggleFavorite(path: string | undefined) {
    if (!path) return;
    path = profileKey(path);
    const index = profile.value.favorites.indexOf(path);
    const value = index < 0;
    if (value) profile.value.favorites.push(path);
    else profile.value.favorites.splice(index, 1);
    enqueue({ type: "setFavorite", key: path, value });
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
    enqueue({ type: "setLyricsDisabled", key: path, value: disabled });
    scheduleSave();
  }

  function toggleLyricsDisabled(path: string | undefined) {
    if (!path) return;
    setLyricsDisabled(path, !isLyricsDisabled(path));
  }

  function createPlaylist(name: string): Playlist {
    const playlist: Playlist = { id: createId(), name: name.trim() || "Playlist", tracks: [] };
    profile.value.playlists.push(playlist);
    enqueue({ type: "createPlaylist", playlist: JSON.parse(JSON.stringify(playlist)) as Playlist });
    scheduleSave();
    return playlist;
  }

  function renamePlaylist(id: string, name: string) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    playlist.name = name.trim() || playlist.name;
    enqueue({ type: "renamePlaylist", id, name: playlist.name });
    scheduleSave();
  }

  /** Update mutable playlist fields (name, cover, tracks and/or smart metadata). */
  function updatePlaylist(id: string, patch: Partial<Pick<Playlist, "name" | "cover" | "coverFile" | "coverTrack" | "tracks" | "smartTransition">>) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    if (patch.name !== undefined) playlist.name = patch.name.trim() || playlist.name;
    if (patch.tracks !== undefined) playlist.tracks = [...patch.tracks];
    if ("cover" in patch) playlist.cover = patch.cover;
    if ("coverFile" in patch) playlist.coverFile = patch.coverFile;
    if ("coverTrack" in patch) playlist.coverTrack = patch.coverTrack;
    if ("smartTransition" in patch) playlist.smartTransition = patch.smartTransition;
    const op: PlaylistPatch = {};
    if (patch.name !== undefined) op.name = playlist.name;
    if (patch.tracks !== undefined) op.tracks = [...patch.tracks];
    if ("cover" in patch || "coverFile" in patch) {
      if (patch.cover === undefined && patch.coverFile === undefined) op.clearCover = true;
      else {
        op.cover = patch.cover;
        op.coverFile = patch.coverFile;
      }
    }
    if ("coverTrack" in patch) {
      if (patch.coverTrack === undefined) op.clearCoverTrack = true;
      else op.coverTrack = patch.coverTrack;
    }
    if ("smartTransition" in patch) {
      if (patch.smartTransition === undefined) op.clearSmartTransition = true;
      else op.smartTransition = patch.smartTransition;
    }
    enqueue({ type: "updatePlaylist", id, patch: op });
    scheduleSave();
  }

  /** Mark a playlist's analysis as stale after its track list changes. */
  function markSmartTransitionStale(id: string) {
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist?.smartTransition || playlist.smartTransition.status !== "ready") return;
    playlist.smartTransition = { ...playlist.smartTransition, status: "stale" };
    enqueue({ type: "markSmartTransitionStale", id });
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
    // Defer deletion until the profile merge has reached the cloud; otherwise a
    // conflict could leave the other version referencing a now-missing file.
    pendingCoverPrunes.add(file);
  }

  /** Actually delete queued cover objects that nothing references any more. */
  async function runCoverPrunes(): Promise<void> {
    if (pendingCoverPrunes.size === 0) return;
    const files = [...pendingCoverPrunes];
    pendingCoverPrunes.clear();
    for (const file of files) {
      if (profile.value.playlists.some((item) => item.coverFile === file)) continue;
      const next = { ...coverUrls.value };
      delete next[file];
      coverUrls.value = next;
      try {
        await invoke("delete_playlist_cover", { file });
      } catch (error) {
        console.warn("[welkin] failed to delete playlist cover", error);
      }
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
        enqueue({ type: "updatePlaylist", id: playlist.id, patch: { clearCover: true, coverFile: file } });
      } catch (error) {
        console.warn("[welkin] failed to migrate playlist cover", error);
      }
    }
    touch();
    scheduleSave();
  }

  function deletePlaylist(id: string) {
    const file = profile.value.playlists.find((item) => item.id === id)?.coverFile;
    const smart = profile.value.playlists.find((item) => item.id === id)?.smartTransition;
    profile.value.playlists = profile.value.playlists.filter((item) => item.id !== id);
    enqueue({ type: "deletePlaylist", id });
    scheduleSave();
    if (file) void pruneCover(file);
    if (smart) {
      // Best-effort: the document is only useful for this playlist.
      void deleteSmartTransition(id).catch((error) => {
        console.warn("[welkin] failed to delete smart transition", error);
      });
    }
  }

  function addToPlaylist(id: string, path: string | undefined) {
    if (!path) return;
    path = profileKey(path);
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist || playlist.tracks.includes(path)) return;
    playlist.tracks.push(path);
    markSmartTransitionStale(id);
    enqueue({ type: "addTrack", id, key: path });
    scheduleSave();
  }

  function removeFromPlaylist(id: string, path: string) {
    path = profileKey(path);
    const playlist = profile.value.playlists.find((item) => item.id === id);
    if (!playlist) return;
    playlist.tracks = playlist.tracks.filter((item) => item !== path);
    markSmartTransitionStale(id);
    enqueue({ type: "removeTrack", id, key: path });
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
    markSmartTransitionStale(id);
    enqueue({ type: "moveTrack", id, key: path, direction });
    scheduleSave();
  }

  function getTrackIndexInPlaylist(id: string, path: string): { index: number; total: number } {
    path = profileKey(path);
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
    conflict,
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
    resolveConflict,
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
    markSmartTransitionStale,
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
