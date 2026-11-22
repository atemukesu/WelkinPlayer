/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke, toAppError } from "../api";
import { pushToast } from "../lib/toast";
import { formatDuration, coverUrl, trackFromEntry, folderOfEntry } from "../lib/remote";
import type { RemoteEntry, TrackMetadata, CachedTrack, TrackSourceRef } from "../lib/remote";
import { trackKey } from "../lib/sources";
import type { SongSource } from "../lib/sources";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";
import { useSourcesStore } from "../stores/sources";

const ERROR_KEYS: Record<string, string> = {
  UNAUTHORIZED: "errors.unauthorized", FORBIDDEN: "errors.forbidden", NOT_FOUND: "errors.notFound",
  TIMEOUT: "errors.timeout", DNS: "errors.dns", TLS: "errors.tls", CONNECTION: "errors.connection",
  HTTP: "errors.http", XML: "errors.xml", MISSING_CREDENTIALS: "errors.missingCredentials",
  INSECURE_URL: "errors.insecureUrl",
  STORE: "errors.store", INVALID_ARGUMENT: "errors.invalidArgument",
};

/** Mirrors the Rust `RefreshResult` struct. */
interface RefreshResult {
  entries: RemoteEntry[];
  changed: boolean;
  changedPaths: string[];
  /** The scan was incomplete/unsafe, so the on-disk cache was left untouched. */
  protected: boolean;
  failedPaths: string[];
  /** `"incomplete"` or `"shrink"` when `protected`. */
  warning: string | null;
}

/** One source's freshly listed (or cached) entries. */
interface SourceEntries {
  source: SongSource;
  entries: RemoteEntry[];
}

export function useLibrary() {
  const { t } = useI18n();
  const player = usePlayerStore();
  const sources = useSourcesStore();
  const loadingLibrary = ref(false);
  const refreshing = ref(false);

  function friendlyError(error: unknown): string {
    const appError = toAppError(error);
    const key = ERROR_KEYS[appError.code];
    return key ? t(key) : appError.message;
  }

  function sourceRef(source: SongSource): TrackSourceRef {
    return { id: source.id, name: source.name, kind: source.kind };
  }

  /** A label that tells identical-looking songs apart: source + folder. */
  function originLabel(source: SongSource, entry: RemoteEntry): string {
    const folder = folderOfEntry(entry);
    return folder ? `${source.name} · ${folder}` : source.name;
  }

  /**
   * Identity of a physical file. The same name and byte size means the same
   * copy, so this is what marks two rows as duplicates (they are still shown,
   * never collapsed: the user may want both a local and a cloud copy).
   */
  function duplicateKey(entry: RemoteEntry): string | null {
    const name = (entry.name || entry.localPath.split("/").pop() || "").trim().toLocaleLowerCase();
    const size = entry.size;
    return name && size && size > 0 ? `${name}\u0000${size}` : null;
  }

  /** Merge per-source entries into one library, reusing cached track objects. */
  function mergeTracks(groups: SourceEntries[], invalidate: Set<string>): Track[] {
    const cachedByKey = new Map(
      player.tracks
        .filter((track) => track.path)
        .map((track) => [trackKey(track) as string, track]),
    );
    // Locate real copies before building rows: every file whose name and byte
    // size match another file gets a "duplicate" hint with the other location.
    const copies = new Map<string, Array<{ id: string; label: string }>>();
    for (const { source, entries } of groups) {
      for (const entry of entries) {
        const key = duplicateKey(entry);
        if (!key) continue;
        const list = copies.get(key) ?? [];
        list.push({
          id: `${source.id}::${entry.localPath || entry.path}`,
          label: originLabel(source, entry),
        });
        copies.set(key, list);
      }
    }

    const merged: Track[] = [];
    // Exact duplicates of the *same object* (same source and same decoded path,
    // whatever href spelling the server used) are the only rows collapsed.
    const seen = new Set<string>();
    for (const { source, entries } of groups) {
      for (const entry of entries) {
        const objectKey = `${source.id}::${entry.localPath || entry.path}`;
        if (seen.has(objectKey)) continue;
        seen.add(objectKey);
        const pathKey = `${source.id}::${entry.path}`;
        const previous = invalidate.has(pathKey) ? undefined : cachedByKey.get(pathKey);
        const track = previous ? { ...previous } : trackFromEntry(entry, merged.length, sourceRef(source));
        track.sourceId = source.id;
        track.sourceName = source.name;
        track.folder = folderOfEntry(entry);
        // Keep the timestamps in sync with the fresh listing even when the
        // track object (and its loaded metadata) is reused from the cache.
        track.modified = entry.modified;
        track.created = entry.created;
        const duplicateOf = duplicateKey(entry);
        const others = duplicateOf
          ? [...new Set((copies.get(duplicateOf) ?? [])
              .filter((copy) => copy.id !== objectKey)
              .map((copy) => copy.label))]
          : [];
        if (others.length > 0) track.duplicates = others;
        else delete track.duplicates;
        merged.push(track);
      }
    }
    merged.forEach((track, index) => (track.id = index + 1));
    return merged;
  }

  function showTracks(groups: SourceEntries[], invalidate: Set<string> = new Set()): Track[] {
    const tracks = mergeTracks(groups, invalidate);
    player.setTracks(tracks);
    // Render from the on-disk cache only. Anything still missing is fetched
    // lazily by the metadata store once a row/card becomes visible.
    void hydrateCachedAssets(tracks);
    return tracks;
  }

  /** Apply cached metadata + cover URLs to the whole list in batches per source. */
  async function hydrateCachedAssets(tracks: Track[]) {
    const targets = tracks.filter((track) => track.path && track.sourceId && !track.assetsHydrated);
    if (targets.length === 0) return;

    const bySource = new Map<string, Track[]>();
    for (const track of targets) {
      const list = bySource.get(track.sourceId as string) ?? [];
      list.push(track);
      bySource.set(track.sourceId as string, list);
    }

    const patches: Array<{ id: number; patch: Partial<Track> }> = [];
    for (const [sourceId, group] of bySource) {
      const paths = group.map((track) => track.path).filter((path): path is string => !!path);
      let cached: CachedTrack[];
      try { cached = await invoke<CachedTrack[]>("load_cached_tracks", { sourceId, paths }); }
      catch (error) { console.warn("[welkin] cached track load failed", error); continue; }
      const byPath = new Map(cached.map((entry) => [entry.path, entry]));
      for (const track of group) {
        if (!track.path) continue;
        const entry = byPath.get(track.path);
        const patch: Partial<Track> = { assetsHydrated: true };
        const meta = entry?.metadata;
        if (meta) {
          if (meta.title) patch.title = meta.title;
          if (meta.artist) patch.artist = meta.artist;
          if (meta.album) patch.album = meta.album;
          if (meta.durationSecs) patch.duration = formatDuration(meta.durationSecs);
          patch.metaLoaded = true;
        }
        const url = coverUrl(entry?.coverPath, track.modified);
        if (url) patch.cover = url;
        if (meta?.coverHash) patch.coverHash = meta.coverHash;
        patches.push({ id: track.id, patch });
      }
    }
    player.updateTracks(patches);
  }

  async function applyMetadata(track: Track, meta: TrackMetadata) {
    const patch: Partial<Track> = {};
    if (meta.title) patch.title = meta.title;
    if (meta.artist) patch.artist = meta.artist;
    if (meta.album) patch.album = meta.album;
    if (meta.durationSecs) patch.duration = formatDuration(meta.durationSecs);
    if (Object.keys(patch).length > 0) player.updateTrack(track.id, patch);

    const resolved: Partial<Track> = { metaLoaded: true };
    if (meta.coverHash) {
      resolved.coverHash = meta.coverHash;
      const path = await invoke<string | null>("cover_path", { hash: meta.coverHash });
      const url = coverUrl(path, track.modified);
      if (url) resolved.cover = url;
    }
    player.updateTrack(track.id, resolved);
  }

  async function downloadTrackMetadata(track: Track) {
    if (!track.path || !track.sourceId) return;
    const metadata = await invoke<TrackMetadata>("download_track_metadata", { sourceId: track.sourceId, path: track.path });
    await applyMetadata(track, metadata);
  }

  /** Load every configured source's cached listing and render it. */
  async function loadCachedLibrary() {
    if (player.tracks.length > 0) return;
    if (!sources.hasSources) return;
    loadingLibrary.value = true;
    try {
      const groups: SourceEntries[] = [];
      for (const source of sources.sources) {
        const entries = await invoke<RemoteEntry[]>("load_library_cache", { sourceId: source.id }).catch(() => []);
        if (entries.length > 0) groups.push({ source, entries });
      }
      if (groups.length > 0) showTracks(groups);
    } finally { loadingLibrary.value = false; }
  }

  /** Refresh every source, falling back to its cache when unreachable. */
  async function refreshAll(options?: { silent?: boolean }): Promise<{ changed: boolean; any: boolean; protected: boolean }> {
    const groups: SourceEntries[] = [];
    const invalidate = new Set<string>();
    let changed = false;
    let anyError: unknown = null;
    let protectedResult = false;
    let warning: string | null = null;

    for (const source of sources.sources) {
      try {
        const result = await invoke<RefreshResult>("refresh_source_library", { sourceId: source.id });
        changed = changed || result.changed;
        protectedResult = protectedResult || result.protected;
        if (result.protected) warning = result.warning;
        groups.push({ source, entries: result.entries });
        for (const path of result.changedPaths) invalidate.add(`${source.id}::${path}`);
      } catch (error) {
        anyError = error;
        const cached = await invoke<RemoteEntry[]>("load_library_cache", { sourceId: source.id }).catch(() => []);
        if (cached.length > 0) groups.push({ source, entries: cached });
      }
    }

    if (groups.length === 0) {
      if (anyError && !options?.silent) pushToast("error", friendlyError(anyError));
      return { changed: false, any: false, protected: false };
    }
    showTracks(groups, invalidate);
    if (anyError && !options?.silent) pushToast("info", t("library.usingCache"));
    else if (warning && !options?.silent) {
      pushToast("info", t(warning === "incomplete" ? "library.partialListing" : "library.cacheProtected"));
    }
    return { changed, any: true, protected: protectedResult };
  }

  async function loadRemoteLibrary(options?: { silent?: boolean }) {
    if (!sources.hasSources) return;
    loadingLibrary.value = true;
    try {
      if (player.tracks.length === 0) await loadCachedLibrary();
      await refreshAll(options);
    } finally { loadingLibrary.value = false; }
  }

  /** Force an immediate re-list of every source. */
  async function refreshLibrary() {
    if (!sources.hasSources) return;
    refreshing.value = true;
    try {
      const { changed, protected: wasProtected } = await refreshAll();
      // A protected refresh already showed why the cache was kept.
      if (!wasProtected) pushToast(changed ? "success" : "info", changed ? t("library.refreshed") : t("library.unchanged"));
    } finally {
      refreshing.value = false;
    }
  }

  return { loadingLibrary, refreshing, friendlyError, loadCachedLibrary, loadRemoteLibrary, refreshLibrary, downloadTrackMetadata };
}
