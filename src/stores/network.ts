/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";

/** How often the active network type is re-checked. */
const POLL_MS = 20000;

/**
 * Active-network classification (Wi-Fi vs metered). Desktop always reports
 * unmetered; Android queries the system. Speculative downloads are gated on
 * this, so the badge and the prefetch logic share one source of truth.
 */
export const useNetworkStore = defineStore("network", () => {
  const metered = ref(false);
  const wifi = ref(true);
  const available = ref(true);
  let timer = 0;

  async function refresh() {
    try {
      const status = await invoke<{ metered: boolean; wifi: boolean; available: boolean }>("network_status");
      metered.value = status.metered;
      wifi.value = status.wifi;
      available.value = status.available;
    } catch {
      // Keep the permissive default; a failed probe must not block playback.
    }
  }

  /** Whether speculative downloads may spend this connection. */
  function isUnmetered(): boolean {
    return !metered.value;
  }

  function start() {
    void refresh();
    if (!timer) timer = window.setInterval(() => void refresh(), POLL_MS);
  }

  return { metered, wifi, available, isUnmetered, refresh, start };
});
