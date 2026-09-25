import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke, toAppError } from "../api";
import { pushToast } from "../lib/toast";
import { formatDuration, trackFromEntry } from "../lib/remote";
import type { RemoteEntry, TrackMetadata } from "../lib/remote";
import { usePlayerStore } from "../stores/player";
import type { Track } from "../stores/player";

const ERROR_KEYS: Record<string, string> = {
  UNAUTHORIZED: "errors.unauthorized", FORBIDDEN: "errors.forbidden", NOT_FOUND: "errors.notFound",
  TIMEOUT: "errors.timeout", DNS: "errors.dns", TLS: "errors.tls", CONNECTION: "errors.connection",
  HTTP: "errors.http", XML: "errors.xml", MISSING_CREDENTIALS: "errors.missingCredentials",
  STORE: "errors.store", INVALID_ARGUMENT: "errors.invalidArgument",
};

export function useLibrary() {
  const { t } = useI18n();
  const player = usePlayerStore();
  const loadingLibrary = ref(false);
  const enriching = ref(false);
  const enrichDone = ref(0);
  const enrichTotal = ref(0);

  function friendlyError(error: unknown): string {
    const appError = toAppError(error);
    const key = ERROR_KEYS[appError.code];
    return key ? t(key) : appError.message;
  }

  function reconcileTracks(entries: RemoteEntry[], cached: Track[]): Track[] {
    const byPath = new Map(cached.map((track) => [track.path, track]));
    return entries.map((entry, index) => {
      const previous = byPath.get(entry.path);
      return previous ? { ...previous, id: index + 1 } : trackFromEntry(entry, index);
    });
  }

  function showTracks(entries: RemoteEntry[], cached: Track[]): Track[] {
    const tracks = reconcileTracks(entries, cached);
    player.setTracks(tracks);
    void enrichTracks(tracks);
    return tracks;
  }

  async function applyMetadata(track: Track, meta: TrackMetadata) {
    const patch: Partial<Track> = {};
    if (meta.title) patch.title = meta.title;
    if (meta.artist) patch.artist = meta.artist;
    if (meta.album) patch.album = meta.album;
    if (meta.durationSecs) patch.duration = formatDuration(meta.durationSecs);
    player.updateTrack(track.id, patch);
    if (meta.coverHash) {
      const cover = await invoke<string | null>("get_cover", { hash: meta.coverHash });
      if (cover) player.updateTrack(track.id, { cover });
    }
  }

  async function enrichTracks(tracks: Track[]) {
    const queue = tracks.filter((track) => track.path);
    if (queue.length === 0) return;
    enriching.value = true;
    enrichDone.value = 0;
    enrichTotal.value = queue.length;
    const worker = async () => {
      for (let track = queue.shift(); track; track = queue.shift()) {
        if (!track.path) continue;
        try {
          const cached = await invoke<TrackMetadata | null>("get_cached_metadata", { path: track.path });
          await applyMetadata(track, cached ?? await invoke<TrackMetadata>("read_track_metadata", { path: track.path }));
        } catch (error) {
          console.warn(`[welkin] metadata failed for ${track.path}`, error);
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

  async function loadRemoteLibrary() {
    loadingLibrary.value = true;
    try {
      let current = player.tracks;
      if (current.length === 0) {
        const cached = await invoke<RemoteEntry[]>("load_library_cache").catch(() => []);
        if (cached.length > 0) current = showTracks(cached, []);
      }
      let entries: RemoteEntry[];
      try { entries = await invoke<RemoteEntry[]>("list_webdav_audio", { path: null }); }
      catch (error) {
        pushToast(player.tracks.length === 0 ? "error" : "info", player.tracks.length === 0 ? friendlyError(error) : t("library.usingCache"));
        return;
      }
      if (entries.length === 0) {
        pushToast("info", player.tracks.length === 0 ? t("settings.webdav.libraryEmpty") : t("library.usingCache"));
        return;
      }
      showTracks(entries, current);
      void invoke("save_library_cache", { entries }).catch(() => {});
    } catch (error) { pushToast("error", friendlyError(error)); }
    finally { loadingLibrary.value = false; }
  }

  return { loadingLibrary, enriching, enrichDone, enrichTotal, friendlyError, loadCachedLibrary, loadRemoteLibrary, downloadTrackMetadata };
}
