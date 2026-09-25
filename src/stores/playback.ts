import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";

/** Resume point persisted independently of the profile. */
interface PlaybackState {
  path: string;
  position: number;
  updatedAt: number;
}

const SAVE_DEBOUNCE_MS = 900;

export const usePlaybackStore = defineStore("playback", () => {
  const path = ref<string | null>(null);
  const position = ref(0);
  const ready = ref(false);

  let saveTimer = 0;

  /**
   * Load the locally cached resume point instantly, then refresh from the
   * authoritative remote copy in the background. When neither exists, seed from
   * the profile's legacy `lastTrack`/`lastPosition` once and push it out.
   */
  async function hydrate(seed?: { path?: string; position?: number }): Promise<boolean> {
    if (ready.value) return false;
    let seeded = false;
    try {
      const local = await invoke<PlaybackState | null>("load_local_playback");
      if (local && local.path) {
        path.value = local.path;
        position.value = Math.max(0, local.position);
      } else if (seed?.path) {
        path.value = seed.path;
        position.value = Math.max(0, seed.position ?? 0);
        seeded = true;
        void persist(true);
      }
    } catch (error) {
      console.warn("[welkin] failed to load local playback state", error);
    } finally {
      ready.value = true;
    }
    void syncRemote();
    return seeded;
  }

  /** Replace the cached resume point when the remote copy has one. */
  async function syncRemote(): Promise<void> {
    try {
      const remote = await invoke<PlaybackState | null>("load_remote_playback");
      if (remote && remote.path) {
        path.value = remote.path;
        position.value = Math.max(0, remote.position);
      }
    } catch (error) {
      console.warn("[welkin] failed to load remote playback state", error);
    }
  }

  async function persist(remote: boolean): Promise<void> {
    const current = path.value;
    if (!current) return;
    try {
      await invoke("save_playback", { path: current, position: position.value, remote });
    } catch (error) {
      console.warn("[welkin] failed to save playback state", error);
    }
  }

  /** Update the resume point and schedule a cheap local write. */
  function record(nextPath: string | undefined, nextPosition: number) {
    if (!nextPath) return;
    path.value = nextPath;
    position.value = Math.max(0, Math.round(nextPosition));
    window.clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => void persist(false), SAVE_DEBOUNCE_MS);
  }

  /** Write immediately, including the remote copy (pause / seek / exit). */
  function flush(): Promise<void> {
    window.clearTimeout(saveTimer);
    return persist(true);
  }

  return { path, position, ready, hydrate, record, flush };
});
