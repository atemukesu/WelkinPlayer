import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { trackKey } from "../lib/sources";

const VOLUME_STORAGE_KEY = "welkin-volume";
const MUTED_STORAGE_KEY = "welkin-muted";
const DEFAULT_VOLUME = 72;
const SHUFFLE_STORAGE_KEY = "welkin-shuffle";
const REPEAT_STORAGE_KEY = "welkin-repeat";
export type RepeatMode = "off" | "all" | "one";

/** Restore the locally saved output volume, falling back to the default. */
function readStoredVolume(): number {
  const stored = Number(localStorage.getItem(VOLUME_STORAGE_KEY));
  return Number.isFinite(stored) && stored >= 0 && stored <= 100 ? stored : DEFAULT_VOLUME;
}

export interface Track {
  id: number;
  title: string;
  artist: string;
  album: string;
  duration: string;
  color: string;
  /** Path inside its source, used to fetch metadata, covers and the stream. */
  path?: string;
  /** Id of the source this track was read from. */
  sourceId?: string;
  /** Display name of the source (kept denormalized for the UI). */
  sourceName?: string;
  /** Source last-modified timestamp, used to bust the cover image cache. */
  modified?: string | null;
  /** Cover thumbnail URL (asset protocol), once resolved. */
  cover?: string;
  /** Hex cache id of the cover thumbnail, for native media controls. */
  coverHash?: string;
  /** Whether metadata has already been loaded from cache or the network. */
  metaLoaded?: boolean;
  /** Whether a metadata fetch was attempted and returned nothing (no retry UI). */
  metaFailed?: boolean;
  /** Whether the on-disk cache has already been consulted for this track. */
  assetsHydrated?: boolean;
}

function formatClock(seconds: number): string {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export const usePlayerStore = defineStore("player", () => {
  const tracks = ref<Track[]>([]);
  const queue = ref<Track[]>([]);
  /** Unshuffled source order of `queue`, so toggling shuffle off can restore it. */
  const baseQueue = ref<Track[]>([]);
  const queueIndex = ref(-1);
  /** Bumped when a track is (re)selected, so audio restarts even for the same path. */
  const playbackNonce = ref(0);
  const shuffle = ref(localStorage.getItem(SHUFFLE_STORAGE_KEY) === "1");
  const repeat = ref<RepeatMode>(localStorage.getItem(REPEAT_STORAGE_KEY) === "all" ? "all" : localStorage.getItem(REPEAT_STORAGE_KEY) === "one" ? "one" : "off");

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
  watch(shuffle, (value) => localStorage.setItem(SHUFFLE_STORAGE_KEY, value ? "1" : "0"));
  watch(repeat, (value) => localStorage.setItem(REPEAT_STORAGE_KEY, value));

  // Keep `queueIndex` aligned with the current track whenever it changes from
  // outside the queue helpers (e.g. startup auto-select or resume). This makes
  // next/previous and the queue panel correct from the very first interaction.
  watch(
    () => currentTrack.value?.id,
    (id) => {
      if (id === undefined) return;
      const index = queue.value.findIndex((track) => track.id === id);
      if (index >= 0) queueIndex.value = index;
    },
  );

  const currentIndex = computed(() => {
    if (queueIndex.value >= 0 && queue.value[queueIndex.value]?.id === currentTrack.value?.id) {
      return queueIndex.value + 1;
    }
    return currentTrack.value ? tracks.value.findIndex((track) => track.id === currentTrack.value?.id) + 1 : 0;
  });
  const hasTrack = computed(() => currentTrack.value !== null);
  const elapsedTime = computed(() => formatClock(position.value));
  const totalTime = computed(() =>
    duration.value > 0 ? formatClock(duration.value) : currentTrack.value?.duration ?? "--:--",
  );
  const remainingTime = computed(() =>
    duration.value > 0 ? formatClock(Math.max(0, duration.value - position.value)) : totalTime.value,
  );

  function resetProgress() {
    position.value = 0;
    progress.value = 0;
    bufferedProgress.value = 0;
    duration.value = 0;
  }

  function selectTrack(track: Track) {
    // Re-selecting the same track must still restart playback (repeat-one,
    // clicking the current row again, wrap-around with a one-track queue). The
    // path is the audio identity, so compare on it (ids can be reassigned on
    // a library refresh).
    const restarting = !!track.path && !!currentTrack.value?.path && trackKey(currentTrack.value) === trackKey(track);
    currentTrack.value = track;
    if (restarting) {
      // Same media element: no metadata event will follow, so keep the known
      // duration/buffer and only rewind the position.
      position.value = 0;
      progress.value = 0;
    } else {
      resetProgress();
    }
    isPlaying.value = true;
    if (restarting) playbackNonce.value += 1;
  }

  function shuffled<T>(items: T[]): T[] {
    const result = [...items];
    for (let index = result.length - 1; index > 0; index--) {
      const other = Math.floor(Math.random() * (index + 1));
      [result[index], result[other]] = [result[other], result[index]];
    }
    return result;
  }

  /** Start a track while preserving the exact context it was chosen from. */
  function playInQueue(items: Track[], track: Track) {
    const valid = items.filter((item) => item.path);
    if (valid.length === 0) return;
    const selected = valid.find((item) => item.id === track.id);
    baseQueue.value = [...valid];
    if (shuffle.value && selected) {
      // Keep the explicitly chosen track first, then shuffle the rest, so the
      // starting point is predictable instead of landing at a random offset.
      queue.value = [selected, ...shuffled(valid.filter((item) => item.id !== selected.id))];
      queueIndex.value = 0;
    } else {
      const ordered = shuffle.value ? shuffled(valid) : valid;
      const selectedIndex = selected ? ordered.findIndex((item) => item.id === selected.id) : -1;
      queue.value = ordered;
      queueIndex.value = selectedIndex >= 0 ? selectedIndex : 0;
    }
    selectTrack(queue.value[queueIndex.value]);
  }

  /** Reorder `queue` for the requested shuffle state, keeping `baseQueue` intact. */
  function setShuffle(on: boolean) {
    shuffle.value = on;
    if (queue.value.length === 0) return;
    const current = currentTrack.value;
    if (on) {
      const rest = queue.value.filter((track) => track.id !== current?.id);
      queue.value = current ? [current, ...shuffled(rest)] : shuffled(queue.value);
      queueIndex.value = current ? 0 : -1;
      return;
    }
    // Restore the original source order and re-locate the current track in it.
    const base = baseQueue.value.length > 0 ? baseQueue.value : queue.value;
    queue.value = [...base];
    const index = current ? queue.value.findIndex((track) => track.id === current.id) : -1;
    queueIndex.value = index >= 0 ? index : 0;
  }

  function toggleShuffle() {
    if (repeat.value === "one") repeat.value = "all";
    setShuffle(!shuffle.value);
  }

  function cycleRepeat() {
    const next: RepeatMode = repeat.value === "off" ? "all" : repeat.value === "all" ? "one" : "off";
    repeat.value = next;
    if (next === "one" && shuffle.value) setShuffle(false);
  }

  /** Replace the whole library, e.g. after loading a remote WebDAV listing. */
  function setTracks(nextTracks: Track[]) {
    const previousKey = trackKey(currentTrack.value);
    tracks.value = [...nextTracks];

    // Re-point existing queue entries at the fresh track objects (metadata,
    // covers and ids may have changed) while preserving the user's queue order.
    const byKey = new Map(
      nextTracks
        .filter((track) => track.path)
        .map((track) => [trackKey(track) as string, track]),
    );
    const remap = (list: Track[]) =>
      list
        .map((track) => {
          const key = trackKey(track);
          return key ? byKey.get(key) : undefined;
        })
        .filter((track): track is Track => !!track);

    // A fresh library load must honour the persisted shuffle state so launching
    // with shuffle already on starts with a shuffled queue (the resume watcher
    // only re-points `currentTrack`, it never rebuilds the order). Only the
    // first population/fallback re-shuffles; refreshes keep the running order
    // because `queue` is non-empty by then.
    const source = nextTracks.filter((track) => track.path);
    if (baseQueue.value.length === 0) baseQueue.value = [...source];
    else {
      const remapped = remap(baseQueue.value);
      baseQueue.value = remapped.length > 0 ? remapped : [...source];
    }
    if (queue.value.length === 0) queue.value = shuffle.value ? shuffled(source) : [...source];
    else {
      const remapped = remap(queue.value);
      queue.value = remapped.length > 0 ? remapped : shuffle.value ? shuffled(source) : [...source];
    }

    // Keep the current selection (and its restored position/playing state)
    // when it still exists in the new listing; only fall back to the first
    // track for a genuinely fresh library.
    const preserved = previousKey
      ? nextTracks.find((track) => trackKey(track) === previousKey)
      : undefined;
    if (preserved) {
      currentTrack.value = preserved;
      queueIndex.value = queue.value.findIndex((track) => track.id === preserved.id);
      return;
    }
    currentTrack.value = nextTracks[0] ?? null;
    resetProgress();
    isPlaying.value = false;
    queueIndex.value = currentTrack.value
      ? queue.value.findIndex((track) => track.id === currentTrack.value?.id)
      : -1;
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
    if (queue.value.length === 0) return;
    if (repeat.value === "one") {
      if (currentTrack.value) selectTrack(currentTrack.value);
      return;
    }
    const nextIndex = queueIndex.value + 1;
    if (nextIndex >= queue.value.length) {
      if (repeat.value !== "all") { isPlaying.value = false; return; }
      // Reaching the end: with repeat-all + shuffle, start a fresh shuffle
      // instead of replaying the original shuffled order.
      if (shuffle.value && queue.value.length > 1) {
        const current = currentTrack.value;
        const reshuffled = shuffled(queue.value);
        if (current && reshuffled[0].id === current.id) {
          [reshuffled[0], reshuffled[1]] = [reshuffled[1], reshuffled[0]];
        }
        queue.value = reshuffled;
      }
      queueIndex.value = 0;
    } else queueIndex.value = nextIndex;
    selectTrack(queue.value[queueIndex.value]);
  }

  function previous() {
    if (queue.value.length === 0) return;
    if (repeat.value === "one") {
      if (currentTrack.value) selectTrack(currentTrack.value);
      return;
    }
    queueIndex.value = queueIndex.value <= 0 ? queue.value.length - 1 : queueIndex.value - 1;
    selectTrack(queue.value[queueIndex.value]);
  }

  return {
    tracks,
    queue,
    queueIndex,
    shuffle,
    repeat,
    playbackNonce,
    currentTrack,
    isPlaying,
    progress,
    bufferedProgress,
    position,
    duration,
    volume,
    muted,
    currentIndex,
    hasTrack,
    elapsedTime,
    totalTime,
    remainingTime,
    selectTrack,
    playInQueue,
    toggleShuffle,
    cycleRepeat,
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
