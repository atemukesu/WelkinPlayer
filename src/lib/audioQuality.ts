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

/** One selectable output device, with the music rates it actually supports. */
export interface DeviceEntry {
  /** Stable id (device name / AudioManager product name). */
  id: string;
  /** Human-readable label. */
  name: string;
  /** Music sample rates this device can take. */
  supportedRates: number[];
  /** `AudioDeviceInfo.getType()` on Android; `0` elsewhere. Lets the UI tag a row
   * as headphones / USB / Bluetooth / built-in speaker. */
  deviceType: number;
}

/** Device capabilities reported by the backend. */
export interface AudioDeviceInfo {
  /** Rate the output stream is currently opened at. */
  currentRate: number;
  /** Music rates the device can take (deduplicated, ascending). */
  supportedRates: number[];
  /** Highest rate on offer (0 when unknown). */
  maxRate: number;
  /** The output devices currently available. */
  devices: DeviceEntry[];
  /** Identifier of the device the stream is bound to. `null` means the engine
   * follows the system default rather than a pinned choice. */
  currentDevice: string | null;
}

/** Last probed device info; `null` until {@link refreshDeviceInfo} runs. */
export const deviceInfo = ref<AudioDeviceInfo | null>(null);

/** The device the user asked for, or `null` to follow the system default.
 * Mirrors the backend's pinned selection; kept in sync by {@link refreshDevices}
 * and the `audio:deviceChanged` event. */
export const currentDevice = ref<string | null>(null);

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
    currentDevice.value = info.currentDevice ?? null;
    return info;
  } catch {
    return null;
  }
}

/**
 * List the output devices currently available and sync the pinned selection. The
 * first entry of the returned list is the system default where distinguishable.
 */
export async function refreshDevices(): Promise<DeviceEntry[]> {
  try {
    const info = await invoke<AudioDeviceInfo>("audio_device_info");
    deviceInfo.value = info;
    currentDevice.value = info.currentDevice ?? null;
    return info.devices ?? [];
  } catch {
    return [];
  }
}

/**
 * Select a specific output device. `deviceId` of `null` (or `undefined`) tells
 * the engine to follow the system default. The engine reopens the stream and
 * keeps the current track playing; the `audio:deviceChanged` event later
 * confirms the new `currentDevice`.
 */
export async function setDevice(deviceId: string | null | undefined) {
  const next = deviceId ?? null;
  currentDevice.value = next;
  try {
    await invoke("audio_set_device", { deviceId: next });
  } catch {
    // The engine emits `audio:deviceChanged` on success; if it failed the
    // previous selection stands and will be re-synced by the next refresh.
  }
}
