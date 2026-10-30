/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { ref } from "vue";
import { invoke } from "../api";

/** Client-local playback quality preference (output sample rate, in Hz). */
const STORAGE_KEY = "welkin-audio-rate";

/**
 * Stored output-rate preference, in Hz. The default is 44.1 kHz until the user
 * picks something else; any value asks the engine to open the output stream at
 * exactly that rate.
 */
export const preferredRate = ref(readStored());

/** Device capabilities reported by the backend. */
export interface AudioDeviceInfo {
  /** Rate the output stream is currently opened at. */
  currentRate: number;
  /** Music rates the device can take (deduplicated, ascending). */
  supportedRates: number[];
  /** Highest rate on offer (0 when unknown). */
  maxRate: number;
}

/** Last probed device info; `null` until {@link refreshDeviceInfo} runs. */
export const deviceInfo = ref<AudioDeviceInfo | null>(null);

/** Whether the output-quality picker modal is open. */
export const qualityModalOpen = ref(false);

/** Render a sample rate as e.g. `44.1 kHz`, or `—` when unknown. */
export function formatRate(rate: number): string {
  if (!Number.isFinite(rate) || rate <= 0) return "—";
  const khz = rate / 1000;
  return `${Number.isInteger(khz) ? khz : khz.toFixed(1)} kHz`;
}

function readStored(): number {
  const value = Number(localStorage.getItem(STORAGE_KEY));
  return Number.isFinite(value) && value > 0 ? Math.round(value) : 44_100;
}

/** Persist and apply a new output-rate preference. */
export async function setPreferredRate(rate: number) {
  const next = Number.isFinite(rate) && rate > 0 ? Math.round(rate) : 44_100;
  preferredRate.value = next;
  localStorage.setItem(STORAGE_KEY, String(next));
  await invoke("audio_set_quality", { rate: next }).catch(() => {});
}

/** Probe the output device for the rates it supports. */
export async function refreshDeviceInfo(): Promise<AudioDeviceInfo | null> {
  try {
    const info = await invoke<AudioDeviceInfo>("audio_device_info");
    deviceInfo.value = info;
    return info;
  } catch {
    return null;
  }
}
