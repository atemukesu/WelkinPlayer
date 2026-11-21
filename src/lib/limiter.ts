/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed, reactive, ref, watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "../api";

/**
 * Device-local look-ahead limiter.
 *
 * The DSP lives in `playback/limiter.rs`; this module owns the three protection
 * presets, the advanced values those presets expand to, persistence, and the
 * meter stream for the graph. Like the EQ and effects it is not part of the
 * synced profile: a phone speaker and a desktop DAC need different headroom.
 */

const STORAGE_KEY = "welkin-audio-limiter";

/** Three user-facing strengths. Each maps to a fixed advanced parameter set. */
export type LimiterMode = "gentle" | "standard" | "strong";

export interface LimiterPreset {
  thresholdDb: number;
  releaseMs: number;
  lookaheadMs: number;
}

export const LIMITER_MODES: Record<LimiterMode, LimiterPreset> = {
  gentle: { thresholdDb: -0.5, releaseMs: 120, lookaheadMs: 3 },
  standard: { thresholdDb: -1.0, releaseMs: 80, lookaheadMs: 5 },
  strong: { thresholdDb: -2.0, releaseMs: 50, lookaheadMs: 5 },
};

export const MODE_ORDER: LimiterMode[] = ["gentle", "standard", "strong"];

/** UI ranges, mirrored by `playback/limiter.rs`. */
export const LIMITER_LIMITS = {
  thresholdDb: { min: -12, max: -0.1, step: 0.1, unit: "dB" },
  releaseMs: { min: 20, max: 250, step: 5, unit: "ms" },
  lookaheadMs: { min: 1, max: 10, step: 0.5, unit: "ms" },
} as const;

/** Attack is derived from look-ahead exactly like the Rust DSP. */
const ATTACK_RATIO = 0.8;

interface LimiterSnapshot {
  enabled: boolean;
  thresholdDb: number;
  releaseMs: number;
  lookaheadMs: number;
}

function clamp(value: unknown, min: number, max: number, fallback: number): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(max, Math.max(min, number)) : fallback;
}

function readStored(): LimiterSnapshot {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as
      | Partial<LimiterSnapshot>
      | null;
    const fallback = LIMITER_MODES.standard;
    return {
      enabled: raw?.enabled !== false,
      thresholdDb: clamp(
        raw?.thresholdDb,
        LIMITER_LIMITS.thresholdDb.min,
        LIMITER_LIMITS.thresholdDb.max,
        fallback.thresholdDb,
      ),
      releaseMs: clamp(
        raw?.releaseMs,
        LIMITER_LIMITS.releaseMs.min,
        LIMITER_LIMITS.releaseMs.max,
        fallback.releaseMs,
      ),
      lookaheadMs: clamp(
        raw?.lookaheadMs,
        LIMITER_LIMITS.lookaheadMs.min,
        LIMITER_LIMITS.lookaheadMs.max,
        fallback.lookaheadMs,
      ),
    };
  } catch {
    return { enabled: true, ...LIMITER_MODES.standard };
  }
}

const stored = readStored();

/** Master switch. Off drops the limiter out of the signal path entirely. */
export const limiterEnabled = ref(stored.enabled);
/** Advanced threshold in dBFS. The ceiling always follows it. */
export const limiterThreshold = ref(stored.thresholdDb);
/** Release time in milliseconds. */
export const limiterRelease = ref(stored.releaseMs);
/** Look-ahead in milliseconds. */
export const limiterLookahead = ref(stored.lookaheadMs);

/** The preset whose advanced values match exactly, or `custom`. */
export const activeLimiterMode = computed<LimiterMode | "custom">(() => {
  const match = MODE_ORDER.find((mode) => {
    const preset = LIMITER_MODES[mode];
    return (
      Math.abs(preset.thresholdDb - limiterThreshold.value) < 0.05 &&
      Math.abs(preset.releaseMs - limiterRelease.value) < 0.5 &&
      Math.abs(preset.lookaheadMs - limiterLookahead.value) < 0.05
    );
  });
  return match ?? "custom";
});

/** Derived attack, shown read-only in the advanced section. */
export const limiterAttackMs = computed(
  () => Math.max(0.2, limiterLookahead.value * ATTACK_RATIO),
);

/** Select a protection strength; overwrites the advanced values. */
export function applyLimiterMode(mode: LimiterMode) {
  const preset = LIMITER_MODES[mode];
  limiterThreshold.value = preset.thresholdDb;
  limiterRelease.value = preset.releaseMs;
  limiterLookahead.value = preset.lookaheadMs;
  commitLimiter();
}

/** Restore the standard preset and re-enable the limiter. */
export function resetLimiter() {
  limiterEnabled.value = true;
  applyLimiterMode("standard");
}

function persist() {
  try {
    const payload: LimiterSnapshot = {
      enabled: limiterEnabled.value,
      thresholdDb: limiterThreshold.value,
      releaseMs: limiterRelease.value,
      lookaheadMs: limiterLookahead.value,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // Storage can be unavailable (private mode); the session still works.
  }
}

async function push() {
  try {
    await invoke("audio_set_limiter", {
      settings: {
        enabled: limiterEnabled.value,
        thresholdDb: limiterThreshold.value,
        releaseMs: limiterRelease.value,
        lookaheadMs: limiterLookahead.value,
      },
    });
  } catch {
    // The engine is optional in a plain browser preview.
  }
}

let pendingPersist: number | undefined;

function schedulePersist() {
  if (pendingPersist !== undefined) window.clearTimeout(pendingPersist);
  pendingPersist = window.setTimeout(() => {
    pendingPersist = undefined;
    persist();
  }, 120);
}

watch([limiterEnabled, limiterThreshold, limiterRelease, limiterLookahead], schedulePersist);

/**
 * Send the current values to the engine. Sliders call this on release (or a
 * keyboard commit) so a drag does not retune the DSP on every step; switches
 * and preset choices commit immediately.
 */
export function commitLimiter() {
  if (pendingPersist !== undefined) {
    window.clearTimeout(pendingPersist);
    pendingPersist = undefined;
  }
  persist();
  void push();
}

/** Replay the stored settings after a webview reload. */
export function initLimiter() {
  persist();
  void push();
}

/** One meter tick from the engine. Peaks are linear amplitudes. */
export interface LimiterMeter {
  inputPeak: number;
  outputPeak: number;
  gainReductionDb: number;
}

/** Latest meter value, for the readout in the graph. */
export const limiterMeter = reactive<LimiterMeter>({
  inputPeak: 0,
  outputPeak: 0,
  gainReductionDb: 0,
});

/**
 * Subscribe to `audio:limiter` for as long as the graph is visible. Returns an
 * unsubscribe function; it is a no-op when the Tauri bridge is unavailable.
 */
export function listenLimiterMeter(onTick: (meter: LimiterMeter) => void): () => void {
  let stop: UnlistenFn | null = null;
  let disposed = false;
  void listen<LimiterMeter>("audio:limiter", (event) => {
    const payload = event.payload;
    if (!payload) return;
    limiterMeter.inputPeak = Number.isFinite(payload.inputPeak) ? payload.inputPeak : 0;
    limiterMeter.outputPeak = Number.isFinite(payload.outputPeak) ? payload.outputPeak : 0;
    limiterMeter.gainReductionDb = Number.isFinite(payload.gainReductionDb)
      ? payload.gainReductionDb
      : 0;
    onTick({ ...limiterMeter });
  })
    .then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    })
    .catch(() => {
      // Plain browser preview: no engine events.
    });
  return () => {
    disposed = true;
    stop?.();
    stop = null;
  };
}
