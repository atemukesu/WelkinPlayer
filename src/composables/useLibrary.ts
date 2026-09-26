import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke, toAppError } from "../api";
import { pushToast } from "../lib/toast";
import { formatDuration, coverUrl, trackFromEntry } from "../lib/remote";
import type { RemoteEntry, TrackMetadata, CachedTrack } from "../lib/remote";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";

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

export function useLibrary() {
  const { t } = useI18n();
  const player = usePlayerStore();
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

  function reconcileTracks(entries: RemoteEntry[], cached: Track[], invalidate?: Set<string>): Track[] {
    const byPath = new Map(cached.map((track) => [track.path, track]));
    return entries.map((entry, index) => {
      const previous = invalidate?.has(entry.path) ? undefined : byPath.get(entry.path);
      return previous ? { ...previous, id: index + 1 } : trackFromEntry(entry, index);
    });
  }

  function showTracks(entries: RemoteEntry[], cached: Track[], invalidate?: Set<string>): Track[] {
    const tracks = reconcileTracks(entries, cached, invalidate);
    player.setTracks(tracks);
    // Render fully from the on-disk cache first (single IPC batch), then only
    // hit the network for whatever is still missing.
    void (async () => { await hydrateCachedAssets(tracks); await enrichTracks(tracks); })();
    return tracks;
  }

  /** Apply cached metadata + cover URLs to the whole list in one batch. */
  async function hydrateCachedAssets(tracks: Track[]) {
    const targets = tracks.filter((track) => track.path && !track.assetsHydrated);
    const paths = targets.map((track) => track.path).filter((path): path is string => !!path);
    if (paths.length === 0) return;
    let cached: CachedTrack[];
    try { cached = await invoke<CachedTrack[]>("load_cached_tracks", { paths }); }
    catch (error) { console.warn("[welkin] cached track load failed", error); return; }
    const byPath = new Map(cached.map((entry) => [entry.path, entry]));
    const patches: Array<{ id: number; patch: Partial<Track> }> = [];
    for (const track of targets) {
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
    player.updateTracks(patches);
  }

  async function applyMetadata(track: Track, meta: TrackMetadata) {
    const patch: Partial<Track> = {};
    if (meta.title) patch.title = meta.title;
    if (meta.artist) patch.artist = meta.artist;
    if (meta.album) patch.album = meta.album;
    if (meta.durationSecs) patch.duration = formatDuration(meta.durationSecs);
    if (Object.keys(patch).length > 0) player.updateTrack(track.id, patch);

    // `metaLoaded` is committed together with the cover so the UI can tell a
    // track that is still resolving apart from one that genuinely has no cover.
    const resolved: Partial<Track> = { metaLoaded: true };
    if (meta.coverHash) {
      const path = await invoke<string | null>("cover_path", { hash: meta.coverHash });
      const url = coverUrl(path, track.modified);
      if (url) resolved.cover = url;
    }
    player.updateTrack(track.id, resolved);
  }

  async function enrichTracks(tracks: Track[]) {
    const queue = tracks.filter((track) => track.path && !track.metaLoaded);
    if (queue.length === 0) return;
    enriching.value = true;
    enrichDone.value = 0;
    enrichTotal.value = queue.length;
    const worker = async () => {
      for (let track = queue.shift(); track; track = queue.shift()) {
        if (!track.path) continue;
        try {
          await applyMetadata(track, await invoke<TrackMetadata>("read_track_metadata", { path: track.path }));
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
    if (!track.path) return;
    const metadata = await invoke<TrackMetadata>("download_track_metadata", { path: track.path });
    await applyMetadata(track, metadata);
  }

  async function loadCachedLibrary() {
    if (player.tracks.length > 0) return;
    loadingLibrary.value = true;
    try {
      const cached = await invoke<RemoteEntry[]>("load_library_cache");
      if (cached.length > 0) showTracks(cached, []);
    } catch { /* No cache available yet. */ } finally { loadingLibrary.value = false; }
  }

  async function loadRemoteLibrary(options?: { silent?: boolean }) {
    loadingLibrary.value = true;
    try {
      let current = player.tracks;
      if (current.length === 0) {
        const cached = await invoke<RemoteEntry[]>("load_library_cache").catch(() => []);
        if (cached.length > 0) current = showTracks(cached, []);
      }
      // Compare the fresh WebDAV listing against the local cache: unchanged
      // tracks keep their cached metadata/covers, changed ones are invalidated.
      let result: RefreshResult;
      try { result = await invoke<RefreshResult>("refresh_webdav_library"); }
      catch (error) {
        if (player.tracks.length === 0) pushToast("error", friendlyError(error));
        else if (!options?.silent) pushToast("info", t("library.usingCache"));
        return;
      }
      if (result.entries.length === 0) {
        if (player.tracks.length === 0) pushToast("info", t("settings.webdav.libraryEmpty"));
        else if (!options?.silent) pushToast("info", t("library.usingCache"));
        return;
      }
      showTracks(result.entries, current, new Set(result.changedPaths));
    } catch (error) { pushToast("error", friendlyError(error)); }
    finally { loadingLibrary.value = false; }
  }

  /** Force an immediate WebDAV re-list, updating the cache only when it changed. */
  async function refreshLibrary() {
    refreshing.value = true;
    try {
      const result = await invoke<RefreshResult>("refresh_webdav_library");
      showTracks(result.entries, player.tracks, new Set(result.changedPaths));
      pushToast(result.changed ? "success" : "info", result.changed ? t("library.refreshed") : t("library.unchanged"));
    } catch (error) {
      pushToast("error", friendlyError(error));
    } finally {
      refreshing.value = false;
    }
  }

  return { loadingLibrary, enriching, refreshing, enrichDone, enrichTotal, friendlyError, loadCachedLibrary, loadRemoteLibrary, refreshLibrary, downloadTrackMetadata };
}
