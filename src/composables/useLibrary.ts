import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke, toAppError } from "../api";
import { pushToast } from "../lib/toast";
import { formatDuration, coverUrl, trackFromEntry } from "../lib/remote";
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
  const enriching = ref(false);
  const refreshing = ref(false);
  const enrichDone = ref(0);
  const enrichTotal = ref(0);

  function friendlyError(error: unknown): string {
    const appError = toAppError(error);
    const key = ERROR_KEYS[appError.code];
    return key ? t(key) : appError.message;
  }

  function sourceRef(source: SongSource): TrackSourceRef {
    return { id: source.id, name: source.name, kind: source.kind };
  }

  /** Merge per-source entries into one library, reusing cached track objects. */
  function mergeTracks(groups: SourceEntries[], invalidate: Set<string>): Track[] {
    const cachedByKey = new Map(
      player.tracks
        .filter((track) => track.path)
        .map((track) => [trackKey(track) as string, track]),
    );
    const merged: Track[] = [];
    for (const { source, entries } of groups) {
      for (const entry of entries) {
        const key = `${source.id}::${entry.path}`;
        const previous = invalidate.has(key) ? undefined : cachedByKey.get(key);
        merged.push(previous ? { ...previous } : trackFromEntry(entry, merged.length, sourceRef(source)));
      }
    }
    merged.forEach((track, index) => (track.id = index + 1));
    return merged;
  }

  function showTracks(groups: SourceEntries[], invalidate: Set<string> = new Set()): Track[] {
    const tracks = mergeTracks(groups, invalidate);
    player.setTracks(tracks);
    // Render fully from the on-disk cache first, then only hit the source for
    // whatever is still missing.
    void (async () => { await hydrateCachedAssets(tracks); await enrichTracks(tracks); })();
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
      const path = await invoke<string | null>("cover_path", { hash: meta.coverHash });
      const url = coverUrl(path, track.modified);
      if (url) resolved.cover = url;
    }
    player.updateTrack(track.id, resolved);
  }

  async function enrichTracks(tracks: Track[]) {
    const queue = tracks.filter((track) => track.path && track.sourceId && !track.metaLoaded);
    if (queue.length === 0) return;
    enriching.value = true;
    enrichDone.value = 0;
    enrichTotal.value = queue.length;
    const worker = async () => {
      for (let track = queue.shift(); track; track = queue.shift()) {
        if (!track.path || !track.sourceId) continue;
        try {
          await applyMetadata(track, await invoke<TrackMetadata>("read_track_metadata", { sourceId: track.sourceId, path: track.path }));
        } catch (error) {
          console.warn(`[welkin] metadata failed for ${track.path}`, error);
          player.updateTrack(track.id, { metaLoaded: true });
        } finally { enrichDone.value += 1; }
      }
    };
    await Promise.all(Array.from({ length: Math.min(4, queue.length) }, worker));
    enriching.value = false;
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
  async function refreshAll(options?: { silent?: boolean }): Promise<{ changed: boolean; any: boolean }> {
    const groups: SourceEntries[] = [];
    const invalidate = new Set<string>();
    let changed = false;
    let anyError: unknown = null;

    for (const source of sources.sources) {
      try {
        const result = await invoke<RefreshResult>("refresh_source_library", { sourceId: source.id });
        changed = changed || result.changed;
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
      return { changed: false, any: false };
    }
    showTracks(groups, invalidate);
    if (anyError && !options?.silent) pushToast("info", t("library.usingCache"));
    return { changed, any: true };
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
      const { changed } = await refreshAll();
      pushToast(changed ? "success" : "info", changed ? t("library.refreshed") : t("library.unchanged"));
    } finally {
      refreshing.value = false;
    }
  }

  return { loadingLibrary, enriching, refreshing, enrichDone, enrichTotal, friendlyError, loadCachedLibrary, loadRemoteLibrary, refreshLibrary, downloadTrackMetadata };
}
