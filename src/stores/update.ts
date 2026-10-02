import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { Channel } from "@tauri-apps/api/core";
import { describeError, invoke } from "../api";

/** Update metadata returned by the Rust `check_update` command. */
export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  version: string | null;
  notes: string | null;
  date: string | null;
  /** Populated on Android only; the `.apk` download URL to open in a browser. */
  downloadUrl: string | null;
}

/** One progress message streamed over the `install_update` channel. */
interface DownloadEvent {
  event: "Started" | "Progress" | "Finished";
  data?: { contentLength?: number | null; chunkLength?: number };
}

/**
 * In-app update state.
 *
 * The heavy lifting (signature verification, download, install) is done in
 * Rust; this store only mirrors the shared state so the boot check and the
 * settings screen stay in sync. On Android there is no in-app installer, so the
 * caller opens [`UpdateInfo.downloadUrl`] instead of running {@link install}.
 */
export const useUpdateStore = defineStore("update", () => {
  const checking = ref(false);
  const installing = ref(false);
  const info = ref<UpdateInfo | null>(null);
  const error = ref<string | null>(null);
  const downloaded = ref(0);
  const total = ref<number | null>(null);
  /** True once at least one check has completed, so the UI can say "up to date". */
  const checked = ref(false);

  const available = computed(() => info.value?.available === true);
  /** Download progress in the 0..1 range, or null while the size is unknown. */
  const progress = computed(() => (total.value && total.value > 0 ? Math.min(1, downloaded.value / total.value) : null));

  /** Ask the backend for an update; rethrows so the caller can surface details. */
  async function check(): Promise<UpdateInfo> {
    checking.value = true;
    error.value = null;
    try {
      info.value = await invoke<UpdateInfo>("check_update");
      checked.value = true;
      return info.value;
    } catch (failure) {
      error.value = describeError(failure);
      throw failure;
    } finally {
      checking.value = false;
    }
  }

  /** Download and install the pending desktop update, restarting on success. */
  async function install(): Promise<void> {
    installing.value = true;
    error.value = null;
    downloaded.value = 0;
    total.value = null;
    try {
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = (message) => {
        if (message.event === "Started") {
          total.value = message.data?.contentLength ?? null;
        } else if (message.event === "Progress") {
          downloaded.value += message.data?.chunkLength ?? 0;
        }
      };
      await invoke("install_update", { onEvent: channel });
    } catch (failure) {
      error.value = describeError(failure);
      throw failure;
    } finally {
      installing.value = false;
    }
  }

  return { checking, installing, info, error, downloaded, total, checked, available, progress, check, install };
});
