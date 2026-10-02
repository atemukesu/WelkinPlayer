/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

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

/** LocalStorage key holding the version the user chose to skip. */
const SKIP_KEY = "welkin-skip-version";

/** Read the skipped version, tolerating storage being unavailable. */
function readSkipped(): string | null {
  try {
    return localStorage.getItem(SKIP_KEY);
  } catch {
    return null;
  }
}

/**
 * In-app update state.
 *
 * The heavy lifting (signature verification, download, install) is done in
 * Rust; this store only mirrors the shared state so the boot check, the update
 * modal and the settings screen stay in sync. On Android there is no in-app
 * installer, so the caller opens [`UpdateInfo.downloadUrl`] instead of running
 * {@link install}.
 */
export const useUpdateStore = defineStore("update", () => {
  const checking = ref(false);
  const installing = ref(false);
  const info = ref<UpdateInfo | null>(null);
  const error = ref<string | null>(null);
  const downloaded = ref(0);
  const total = ref<number | null>(null);
  /** Smoothed download rate in bytes per second, or null while unknown. */
  const speed = ref<number | null>(null);
  /** True once at least one check has completed, so the UI can say "up to date". */
  const checked = ref(false);
  /** Whether the update modal is open. */
  const open = ref(false);
  /** Version the user chose to skip, silenced by automatic checks. */
  const skipped = ref<string | null>(readSkipped());

  const available = computed(() => info.value?.available === true);
  /** Download progress in the 0..1 range, or null while the size is unknown. */
  const progress = computed(() => (total.value && total.value > 0 ? Math.min(1, downloaded.value / total.value) : null));
  /** True when an update should be offered to the user (newer than any skip). */
  const shouldPrompt = computed(() => available.value && info.value?.version !== skipped.value);

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
    speed.value = null;
    let lastSampleAt = 0;
    try {
      const channel = new Channel<DownloadEvent>();
      channel.onmessage = (message) => {
        if (message.event === "Started") {
          total.value = message.data?.contentLength ?? null;
          lastSampleAt = performance.now();
        } else if (message.event === "Progress") {
          const chunk = message.data?.chunkLength ?? 0;
          downloaded.value += chunk;
          const now = performance.now();
          if (lastSampleAt > 0 && now > lastSampleAt) {
            // Instantaneous rate smoothed with an exponential moving average so
            // the readout stays steady despite the varying chunk sizes.
            const instant = (chunk / (now - lastSampleAt)) * 1000;
            speed.value = speed.value === null ? instant : speed.value * 0.7 + instant * 0.3;
          }
          lastSampleAt = now;
        } else if (message.event === "Finished") {
          speed.value = null;
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

  /** Open the update modal. */
  function show() {
    open.value = true;
  }

  /** Close the update modal; any running download continues in the background. */
  function hide() {
    open.value = false;
  }

  /** "Remind me later": dismiss the modal, to be prompted again next launch. */
  function remindLater() {
    hide();
  }

  /** "Skip this version": silence the update until a newer one is released. */
  function skipVersion() {
    skipped.value = info.value?.version ?? null;
    try {
      if (skipped.value) localStorage.setItem(SKIP_KEY, skipped.value);
    } catch {
      // Storage may be unavailable; the in-memory skip still applies.
    }
    hide();
  }

  /** Clear a previous skip, e.g. when the user checks manually. */
  function clearSkip() {
    skipped.value = null;
    try {
      localStorage.removeItem(SKIP_KEY);
    } catch {
      // Ignore storage failures.
    }
  }

  return {
    checking,
    installing,
    info,
    error,
    downloaded,
    total,
    speed,
    checked,
    open,
    skipped,
    available,
    progress,
    shouldPrompt,
    check,
    install,
    show,
    hide,
    remindLater,
    skipVersion,
    clearSkip,
  };
});
