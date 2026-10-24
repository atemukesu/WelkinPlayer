/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import { computed, ref } from "vue";
import { invoke } from "../api";

/** Client-local playback quality preference. */
const STORAGE_KEY = "welkin-audio-rate";

/**
 * Stored output-rate preference, in Hz. `0` means "follow the device", which is
 * the default until the user picks something. Any other value asks the engine
 * to open the output stream at exactly that rate.
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
  /** The user's stored preference (0 = follow the device). */
  preferredRate: number;
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

/**
 * Whether the chosen rate is one the device did not advertise. Asking for a
 * rate the endpoint does not list means the OS has to resample it, which costs
 * CPU and can add distortion without improving anything.
 */
export const rateTooHigh = computed(() => {
  const info = deviceInfo.value;
  const chosen = preferredRate.value;
  if (!info || chosen === 0) return false;
  if (info.supportedRates.length === 0) return false;
  return !info.supportedRates.includes(chosen);
});

function readStored(): number {
  const value = Number(localStorage.getItem(STORAGE_KEY));
  return Number.isFinite(value) && value > 0 ? Math.round(value) : 0;
}

/** Persist and apply a new output-rate preference. */
export async function setPreferredRate(rate: number) {
  const next = Number.isFinite(rate) && rate > 0 ? Math.round(rate) : 0;
  preferredRate.value = next;
  if (next > 0) localStorage.setItem(STORAGE_KEY, String(next));
  else localStorage.removeItem(STORAGE_KEY);
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
