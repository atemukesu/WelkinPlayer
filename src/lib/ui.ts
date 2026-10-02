/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref, watch } from "vue";

export type TrackViewMode = "list" | "grid";

const STORAGE_KEY = "welkin-track-view";

function read(): TrackViewMode {
  return localStorage.getItem(STORAGE_KEY) === "grid" ? "grid" : "list";
}

/** Shared list/grid preference for track collection pages. */
export const trackViewMode = ref<TrackViewMode>(read());

watch(trackViewMode, (value) => localStorage.setItem(STORAGE_KEY, value));

export function setTrackViewMode(mode: TrackViewMode) {
  trackViewMode.value = mode;
}
