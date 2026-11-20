/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed, ref, watch } from "vue";
import { invoke } from "../api";

/**
 * Device-local sound shaping.
 *
 * The equaliser is a whole-bus 12-band graphic EQ. The band list and gain
 * ranges are mirrored by `playback/graphic_eq.rs`; the engine only stores the
 * values, so this module owns the presets, normalisation and persistence.
 * Like the output-quality choice these settings are *not* part of the synced
 * profile: speakers and taste differ per device.
 */

const STORAGE_KEY = "welkin-audio-eq";

/** Centre frequencies of the twelve bands, low to high. */
export const BANDS = [
  31, 63, 125, 250, 500, 1_000, 2_000, 4_000, 8_000, 12_000, 16_000, 20_000,
] as const;

export const BAND_COUNT = BANDS.length;
export const MIN_GAIN = -12;
export const MAX_GAIN = 12;
export const MIN_PREAMP = -24;
export const MAX_PREAMP = 12;

/** One selectable tonal shape, stored as twelve dB values. */
export interface EqualizerPreset {
  id: string;
  gains: number[];
}

/** Built-in presets. Labels live in i18n under `soundSettings.presets.*`. */
export const PRESETS: EqualizerPreset[] = [
  { id: "flat", gains: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
  { id: "pop", gains: [-1, -1, 0, 2, 4, 4, 2, 0, -1, -1, -2, -2] },
  { id: "rock", gains: [5, 4, 3, 1, -1, -1, 1, 3, 4, 4, 3, 3] },
  { id: "jazz", gains: [3, 2, 1, 2, -1, -1, 0, 1, 2, 3, 3, 3] },
  { id: "classical", gains: [4, 3, 2, 1, -1, -1, 0, 2, 3, 3, 4, 4] },
  { id: "bass", gains: [8, 7, 6, 4, 1, 0, -1, -1, 0, 0, 0, 0] },
  { id: "vocal", gains: [-3, -2, -1, 1, 4, 5, 4, 3, 1, 0, -1, -2] },
  { id: "electronic", gains: [6, 5, 3, 0, -1, 1, 2, 3, 5, 5, 4, 4] },
];

interface EqualizerSettings {
  enabled: boolean;
  gains: number[];
  preamp: number;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function normalizeGains(raw: unknown): number[] {
  const source = Array.isArray(raw) ? raw : [];
  return Array.from({ length: BAND_COUNT }, (_, index) => {
    const value = Number(source[index]);
    return Number.isFinite(value) ? clamp(Math.round(value), MIN_GAIN, MAX_GAIN) : 0;
  });
}

function readStored(): EqualizerSettings {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as
      | Partial<EqualizerSettings>
      | null;
    const preamp = Number(raw?.preamp);
    return {
      enabled: raw?.enabled === true,
      gains: normalizeGains(raw?.gains),
      preamp: Number.isFinite(preamp) ? clamp(Math.round(preamp), MIN_PREAMP, MAX_PREAMP) : 0,
    };
  } catch {
    return { enabled: false, gains: normalizeGains(null), preamp: 0 };
  }
}

const stored = readStored();

/** Master switch. Off leaves the audio path completely untouched. */
export const eqEnabled = ref(stored.enabled);
/** Twelve band gains in dB, low to high. */
export const eqGains = ref<number[]>(stored.gains);
/** Pre-amp in dB; negative values leave headroom for boosted bands. */
export const eqPreamp = ref(stored.preamp);
/** Whether the sound-settings floating window is open. */
export const eqWindowOpen = ref(false);

/** Render a band centre as `31`, `250` or `12k`. */
export function formatBand(hz: number): string {
  if (hz < 1_000) return String(hz);
  const khz = hz / 1_000;
  return `${Number.isInteger(khz) ? khz : khz.toFixed(1)}k`;
}

/** Render a gain as `+3`, `0` or `-6`. */
export function formatGain(value: number): string {
  const rounded = Math.round(value);
  return `${rounded > 0 ? "+" : ""}${rounded}`;
}

/** The preset whose gains currently match exactly, or `custom`. */
export const activePreset = computed(() => {
  const match = PRESETS.find((preset) =>
    preset.gains.every((gain, index) => Math.abs(gain - eqGains.value[index]) < 0.01),
  );
  return match?.id ?? "custom";
});

/** Copy a preset's curve into the live gains. `custom` is a no-op. */
export function applyPreset(id: string) {
  const preset = PRESETS.find((entry) => entry.id === id);
  if (preset) {
    eqGains.value = [...preset.gains];
    commitEqualizer();
  }
}

/** Return every band and the pre-amp to neutral. */
export function resetEqualizer() {
  eqGains.value = Array.from({ length: BAND_COUNT }, () => 0);
  eqPreamp.value = 0;
  commitEqualizer();
}

function persist() {
  try {
    const payload: EqualizerSettings = {
      enabled: eqEnabled.value,
      gains: eqGains.value,
      preamp: eqPreamp.value,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // Storage can be unavailable (private mode); the session still works.
  }
}

async function push() {
  try {
    await invoke("audio_set_equalizer", {
      enabled: eqEnabled.value,
      gains: eqGains.value,
      preamp: eqPreamp.value,
    });
  } catch {
    // The engine is optional in a plain browser preview.
  }
}

let pendingPersist: number | undefined;

/** Persist the device-local copy without retuning the live engine. */
function schedulePersist() {
  if (pendingPersist !== undefined) window.clearTimeout(pendingPersist);
  pendingPersist = window.setTimeout(() => {
    pendingPersist = undefined;
    persist();
  }, 120);
}

watch([eqEnabled, eqPreamp, eqGains], schedulePersist, { deep: true });

/**
 * Send the current values to the engine.
 *
 * Sliders call this on `change` (pointer release or a keyboard commit) so a
 * drag updates the on-screen curve without retuning the filters on every step.
 * Toggles and presets are single actions and commit immediately.
 */
export function commitEqualizer() {
  if (pendingPersist !== undefined) {
    window.clearTimeout(pendingPersist);
    pendingPersist = undefined;
  }
  persist();
  void push();
}

/**
 * Replay the stored settings after a webview reload. The Rust engine keeps its
 * own copy for the process lifetime, so this only matters when the UI restarts
 * while audio is already playing.
 */
export function initEqualizer() {
  persist();
  void push();
}
