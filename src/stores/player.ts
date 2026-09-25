import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";

const VOLUME_STORAGE_KEY = "welkin-volume";
const MUTED_STORAGE_KEY = "welkin-muted";
const DEFAULT_VOLUME = 72;
const SAVE_INTERVAL_STORAGE_KEY = "welkin-save-interval";
export const DEFAULT_SAVE_INTERVAL = 15;
export const MIN_SAVE_INTERVAL = 5;
export const MAX_SAVE_INTERVAL = 60;

/** Restore the locally saved output volume, falling back to the default. */
function readStoredVolume(): number {
  const stored = Number(localStorage.getItem(VOLUME_STORAGE_KEY));
  return Number.isFinite(stored) && stored >= 0 && stored <= 100 ? stored : DEFAULT_VOLUME;
}

/** Restore the locally saved progress-save interval (seconds). */
function readStoredSaveInterval(): number {
  const stored = Number(localStorage.getItem(SAVE_INTERVAL_STORAGE_KEY));
  return Number.isFinite(stored) && stored >= MIN_SAVE_INTERVAL && stored <= MAX_SAVE_INTERVAL
    ? Math.round(stored)
    : DEFAULT_SAVE_INTERVAL;
}

export interface Track {
  id: number;
  title: string;
  artist: string;
  album: string;
  duration: string;
  color: string;
  /** Remote WebDAV href, used to fetch metadata, covers and the audio stream. */
  path?: string;
  /** Remote last-modified timestamp, used to bust the cover image cache. */
  modified?: string | null;
  /** Cover thumbnail URL (asset protocol), once resolved. */
  cover?: string;
  /** Whether metadata has already been loaded from cache or the network. */
  metaLoaded?: boolean;
  /** Whether the on-disk cache has already been consulted for this track. */
  assetsHydrated?: boolean;
}

function formatClock(seconds: number): string {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export const usePlayerStore = defineStore("player", () => {
  const tracks = ref<Track[]>([]);

  const currentTrack = ref<Track | null>(null);
  const isPlaying = ref(false);
  /** Playback position as a percentage (drives the progress sliders). */
  const progress = ref(0);
  /** End of the buffered range containing the current position, as a percentage. */
  const bufferedProgress = ref(0);
  /** Playback position in seconds. */
  const position = ref(0);
  /** Track length in seconds, once the media element reports it. */
  const duration = ref(0);
  const volume = ref(readStoredVolume());
  const muted = ref(localStorage.getItem(MUTED_STORAGE_KEY) === "1");
  /** Seconds between periodic progress saves while audio is playing. */
  const saveInterval = ref(readStoredSaveInterval());

  // Persist the output volume/mute locally so each launch starts where the
  // previous one left off (mirrors the theme/accent localStorage pattern).
  // The watcher also clamps, so no code path can ever store a value outside 0-100.
  watch(volume, (value) => {
    const clamped = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : DEFAULT_VOLUME;
    if (clamped !== value) {
      volume.value = clamped;
      return;
    }
    localStorage.setItem(VOLUME_STORAGE_KEY, String(clamped));
  });
  watch(muted, (value) => localStorage.setItem(MUTED_STORAGE_KEY, value ? "1" : "0"));
  watch(saveInterval, (value) => {
    const clamped = Number.isFinite(value)
      ? Math.min(MAX_SAVE_INTERVAL, Math.max(MIN_SAVE_INTERVAL, value))
      : DEFAULT_SAVE_INTERVAL;
    if (clamped !== value) {
      saveInterval.value = clamped;
      return;
    }
    localStorage.setItem(SAVE_INTERVAL_STORAGE_KEY, String(clamped));
  });

  const currentIndex = computed(() =>
    currentTrack.value ? tracks.value.findIndex((track) => track.id === currentTrack.value?.id) + 1 : 0,
  );
  const hasTrack = computed(() => currentTrack.value !== null);
  const elapsedTime = computed(() => formatClock(position.value));
  const totalTime = computed(() =>
    duration.value > 0 ? formatClock(duration.value) : currentTrack.value?.duration ?? "--:--",
  );

  function resetProgress() {
    position.value = 0;
    progress.value = 0;
    bufferedProgress.value = 0;
    duration.value = 0;
  }

  function selectTrack(track: Track) {
    currentTrack.value = track;
    resetProgress();
    isPlaying.value = true;
  }

  /** Replace the whole library, e.g. after loading a remote WebDAV listing. */
  function setTracks(nextTracks: Track[]) {
    const previousPath = currentTrack.value?.path;
    tracks.value = [...nextTracks];
    // Keep the current selection (and its restored position/playing state)
    // when it still exists in the new listing; only fall back to the first
    // track for a genuinely fresh library.
    const preserved = previousPath
      ? nextTracks.find((track) => track.path === previousPath)
      : undefined;
    if (preserved) {
      currentTrack.value = preserved;
      return;
    }
    currentTrack.value = nextTracks[0] ?? null;
    resetProgress();
    isPlaying.value = false;
  }

  /** Merge a partial update into one track (metadata / cover arrive later). */
  function updateTrack(id: number, patch: Partial<Track>) {
    const track = tracks.value.find((item) => item.id === id);
    if (!track) return;
    Object.assign(track, patch);
    if (currentTrack.value && currentTrack.value.id === id) {
      currentTrack.value = { ...currentTrack.value, ...patch };
    }
  }

  /**
   * Merge many partial updates in a single pass. Hydrating a 10k-track library
   * otherwise costs O(n²) `find` calls plus one reactive write per track.
   */
  function updateTracks(patches: Array<{ id: number; patch: Partial<Track> }>) {
    if (patches.length === 0) return;
    const byId = new Map(tracks.value.map((track) => [track.id, track]));
    for (const { id, patch } of patches) {
      const track = byId.get(id);
      if (track) Object.assign(track, patch);
      if (currentTrack.value && currentTrack.value.id === id) {
        currentTrack.value = { ...currentTrack.value, ...patch };
      }
    }
  }

  function togglePlayback() {
    if (!currentTrack.value) return;
    isPlaying.value = !isPlaying.value;
  }

  function setPlaying(value: boolean) {
    isPlaying.value = value;
  }

  /** Update the position from the media element (also syncs the slider). */
  function setPosition(seconds: number) {
    position.value = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
    progress.value = duration.value > 0 ? Math.min(100, (position.value / duration.value) * 100) : 0;
  }

  function setDuration(seconds: number) {
    duration.value = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
    setPosition(position.value);
  }

  function setBufferedProgress(value: number) {
    bufferedProgress.value = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : 0;
  }

  function setVolume(value: number) {
    volume.value = Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : volume.value;
  }

  function toggleMute() {
    muted.value = !muted.value;
  }

  function next() {
    if (tracks.value.length === 0) return;
    const index = tracks.value.findIndex((track) => track.id === currentTrack.value?.id);
    selectTrack(tracks.value[(index + 1) % tracks.value.length]);
  }

  function previous() {
    if (tracks.value.length === 0) return;
    const index = tracks.value.findIndex((track) => track.id === currentTrack.value?.id);
    selectTrack(tracks.value[(index - 1 + tracks.value.length) % tracks.value.length]);
  }

  return {
    tracks,
    currentTrack,
    isPlaying,
    progress,
    bufferedProgress,
    position,
    duration,
    volume,
    muted,
    saveInterval,
    currentIndex,
    hasTrack,
    elapsedTime,
    totalTime,
    selectTrack,
    setTracks,
    updateTrack,
    updateTracks,
    togglePlayback,
    setPlaying,
    setPosition,
    setDuration,
    setBufferedProgress,
    setVolume,
    toggleMute,
    next,
    previous,
  };
});
