/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { reactive, watch } from "vue";
import { invoke } from "../api";

/**
 * Device-local effects box.
 *
 * Five effects share one chain in the playback engine: chorus, flanger, delay,
 * echo and reverb. The defaults and ranges mirror `playback/effects.rs`; the
 * engine only stores clamped values, so this module owns persistence and the
 * normalisation of anything read back from storage.
 */

const STORAGE_KEY = "welkin-audio-effects";

export interface DelaySettings {
  enabled: boolean;
  timeMs: number;
  mix: number;
}

export interface EchoSettings extends DelaySettings {
  feedback: number;
}

export interface ChorusSettings {
  enabled: boolean;
  depth: number;
  mix: number;
}

export interface FlangerSettings extends ChorusSettings {
  feedback: number;
}

export interface ReverbSettings {
  enabled: boolean;
  size: number;
  damping: number;
  mix: number;
}

export interface EffectsSettings {
  delay: DelaySettings;
  echo: EchoSettings;
  chorus: ChorusSettings;
  flanger: FlangerSettings;
  reverb: ReverbSettings;
}

export type FxKey = keyof EffectsSettings;

export const DEFAULT_EFFECTS: EffectsSettings = {
  delay: { enabled: false, timeMs: 250, mix: 30 },
  echo: { enabled: false, timeMs: 350, feedback: 35, mix: 30 },
  chorus: { enabled: false, depth: 50, mix: 40 },
  flanger: { enabled: false, depth: 50, feedback: 30, mix: 40 },
  reverb: { enabled: false, size: 50, damping: 50, mix: 25 },
};

/** UI ranges and units, mirrored by the Rust clamp ranges. */
export const FX_LIMITS = {
  timeMs: { min: 20, max: 1000, step: 10, unit: "ms" },
  mix: { min: 0, max: 100, step: 1, unit: "%" },
  feedback: { min: 0, max: 85, step: 1, unit: "%" },
  depth: { min: 0, max: 100, step: 1, unit: "%" },
  size: { min: 0, max: 100, step: 1, unit: "%" },
  damping: { min: 0, max: 100, step: 1, unit: "%" },
} as const;

function clamp(value: unknown, min: number, max: number, fallback: number): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(max, Math.max(min, number)) : fallback;
}

/** Read one numeric field with clamping; booleans fall back to off. */
function numeric(raw: Record<string, unknown> | undefined, key: string, min: number, max: number, fallback: number): number {
  return clamp(raw?.[key], min, max, fallback);
}

function readStored(): EffectsSettings {
  const base = () => JSON.parse(JSON.stringify(DEFAULT_EFFECTS)) as EffectsSettings;
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null") as
      | Partial<Record<FxKey, Record<string, unknown>>>
      | null;
    if (!raw) return base();
    return {
      delay: {
        enabled: raw.delay?.enabled === true,
        timeMs: numeric(raw.delay, "timeMs", 20, 1000, 250),
        mix: numeric(raw.delay, "mix", 0, 100, 30),
      },
      echo: {
        enabled: raw.echo?.enabled === true,
        timeMs: numeric(raw.echo, "timeMs", 20, 1000, 350),
        feedback: numeric(raw.echo, "feedback", 0, 85, 35),
        mix: numeric(raw.echo, "mix", 0, 100, 30),
      },
      chorus: {
        enabled: raw.chorus?.enabled === true,
        depth: numeric(raw.chorus, "depth", 0, 100, 50),
        mix: numeric(raw.chorus, "mix", 0, 100, 40),
      },
      flanger: {
        enabled: raw.flanger?.enabled === true,
        depth: numeric(raw.flanger, "depth", 0, 100, 50),
        feedback: numeric(raw.flanger, "feedback", 0, 85, 30),
        mix: numeric(raw.flanger, "mix", 0, 100, 40),
      },
      reverb: {
        enabled: raw.reverb?.enabled === true,
        size: numeric(raw.reverb, "size", 0, 100, 50),
        damping: numeric(raw.reverb, "damping", 0, 100, 50),
        mix: numeric(raw.reverb, "mix", 0, 100, 25),
      },
    };
  } catch {
    return base();
  }
}

/** Live effects state. Mutating this updates the UI and schedules persistence. */
export const fx = reactive<EffectsSettings>(readStored());

/** Turn the 0–100 UI values into the 0–1 values the engine expects. */
function toPayload(): EffectsSettings {
  const percent = (value: number) => clamp(value, 0, 100, 0) / 100;
  return {
    delay: {
      enabled: fx.delay.enabled,
      timeMs: clamp(fx.delay.timeMs, 20, 1000, 250),
      mix: percent(fx.delay.mix),
    },
    echo: {
      enabled: fx.echo.enabled,
      timeMs: clamp(fx.echo.timeMs, 20, 1000, 350),
      feedback: percent(fx.echo.feedback),
      mix: percent(fx.echo.mix),
    },
    chorus: {
      enabled: fx.chorus.enabled,
      depth: percent(fx.chorus.depth),
      mix: percent(fx.chorus.mix),
    },
    flanger: {
      enabled: fx.flanger.enabled,
      depth: percent(fx.flanger.depth),
      feedback: percent(fx.flanger.feedback),
      mix: percent(fx.flanger.mix),
    },
    reverb: {
      enabled: fx.reverb.enabled,
      size: percent(fx.reverb.size),
      damping: percent(fx.reverb.damping),
      mix: percent(fx.reverb.mix),
    },
  };
}

function persist() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(fx));
  } catch {
    // Storage can be unavailable (private mode); the session still works.
  }
}

async function push() {
  try {
    await invoke("audio_set_effects", { effects: toPayload() });
  } catch {
    // The engine is optional in a plain browser preview.
  }
}

let pendingPersist: number | undefined;

/** Persist the device-local copy while a slider is being dragged. */
function schedulePersist() {
  if (pendingPersist !== undefined) window.clearTimeout(pendingPersist);
  pendingPersist = window.setTimeout(() => {
    pendingPersist = undefined;
    persist();
  }, 120);
}

watch(fx, schedulePersist, { deep: true });

/**
 * Send the current values to the engine.
 *
 * Switches commit immediately; sliders call this on `change` (pointer release
 * or keyboard commit) so a drag does not reconfigure the chain on every step.
 */
export function commitEffects() {
  if (pendingPersist !== undefined) {
    window.clearTimeout(pendingPersist);
    pendingPersist = undefined;
  }
  persist();
  void push();
}

/**
 * Restore the given effects' knobs to their factory values and apply them.
 *
 * The on/off switches are left alone: "reset" means undoing parameter tweaks,
 * not silently bypassing something the listener just enabled. Effects are flat
 * records, so a shallow copy of the defaults is enough; `DEFAULT_EFFECTS` itself
 * is never mutated.
 */
export function resetEffects(keys: FxKey[]) {
  for (const key of keys) {
    Object.assign(fx[key], DEFAULT_EFFECTS[key], { enabled: fx[key].enabled });
  }
  commitEffects();
}

/**
 * Replay the stored settings after a webview reload, so the engine matches the
 * UI when audio is already playing.
 */
export function initEffects() {
  persist();
  void push();
}
