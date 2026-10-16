/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref, watch } from "vue";

const STORAGE_KEY = "welkin-waveform";

/** Whether the real-time frequency waveform is shown on the player pages. */
export const showWaveform = ref(localStorage.getItem(STORAGE_KEY) === "1");

watch(showWaveform, (value) => localStorage.setItem(STORAGE_KEY, value ? "1" : "0"));
