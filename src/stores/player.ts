import { computed, ref } from "vue";
import { defineStore } from "pinia";

export interface Track {
  id: number;
  title: string;
  artist: string;
  album: string;
  duration: string;
  color: string;
  /** Remote WebDAV href, used to fetch metadata, covers and the audio stream. */
  path?: string;
  /** Cached cover thumbnail as a data URL, once resolved. */
  cover?: string;
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
  const volume = ref(72);
  const muted = ref(false);

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
    tracks.value = [...nextTracks];
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
    volume.value = Math.min(100, Math.max(0, value));
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
    currentIndex,
    hasTrack,
    elapsedTime,
    totalTime,
    selectTrack,
    setTracks,
    updateTrack,
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
