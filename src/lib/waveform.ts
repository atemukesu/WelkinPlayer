/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref, watch } from "vue";

const STORAGE_KEY = "welkin-waveform";
const SCALE_KEY = "welkin-waveform-scale";

/** Waveform height must stay within a sane multiple of the default height. */
const MIN_SCALE = 0.5;
const MAX_SCALE = 3;
const DEFAULT_SCALE = 1;

/** Whether the real-time frequency waveform is shown on the player pages. */
export const showWaveform = ref(localStorage.getItem(STORAGE_KEY) === "1");

/** Multiplier applied to the waveform's default height. */
export const waveformScale = ref(readScale());

function readScale(): number {
  const value = Number(localStorage.getItem(SCALE_KEY));
  if (!Number.isFinite(value) || value <= 0) return DEFAULT_SCALE;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

watch(showWaveform, (value) => localStorage.setItem(STORAGE_KEY, value ? "1" : "0"));
watch(waveformScale, (value) => localStorage.setItem(SCALE_KEY, String(value)));
