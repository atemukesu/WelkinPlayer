<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { spectrumData } from "../lib/audio";
import { showWaveform, waveformScale } from "../lib/waveform";

const props = withDefaults(defineProps<{ barCount?: number; fit?: boolean }>(), {
  barCount: 48,
  fit: false,
});

/** Approximate width (px) of one bar plus its gap when auto-fitting a width. */
const FIT_UNIT = 7;
/** Default waveform height in px (the old `h-8`), scaled by the user setting. */
const DEFAULT_HEIGHT = 32;
/** Matches the old per-bar `opacity: 0.72`. */
const BAR_OPACITY = 0.72;
const MIN_BAR_HEIGHT = 2;
const BAR_WIDTH = 3;
const BAR_GAP = 2;
const FIT_GAP = 4;
/** Matches the old `transition: transform 90ms linear` smoothing. */
const SMOOTH_MS = 90;

/**
 * The spectrum is painted straight into a `<canvas>` rather than through a
 * reactive `v-for`, so the per-frame FFT updates never touch Vue's renderer
 * (no vnode diff, no hundreds of style writes per frame).
 */
const canvas = ref<HTMLCanvasElement | null>(null);
let ctx: CanvasRenderingContext2D | null = null;
let observer: ResizeObserver | null = null;
let themeObserver: MutationObserver | null = null;
let frame = 0;
let lastTime = 0;
let lastW = 0;
let lastH = 0;
let lastDpr = 0;
let color = "#ffffff";
/** Smoothed bar heights, interpolated toward the latest FFT frame. */
let current: number[] = [];

/** Re-read the inherited text colour (callers set e.g. `text-accent`). */
function loadColor() {
  if (canvas.value) color = getComputedStyle(canvas.value).color || "#ffffff";
}

function resize() {
  const el = canvas.value;
  if (!el) return;
  const dpr = window.devicePixelRatio || 1;
  const width = el.clientWidth;
  const height = el.clientHeight;
  if (width <= 0 || height <= 0) return;
  if (width === lastW && height === lastH && dpr === lastDpr) return;
  lastW = width;
  lastH = height;
  lastDpr = dpr;
  el.width = Math.round(width * dpr);
  el.height = Math.round(height * dpr);
  ctx = el.getContext("2d");
  ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
  current = [];
  loadColor();
}

/** Pick a bar count so the bars keep a steady width at any container size. */
function barCountFor(width: number) {
  if (!props.fit) return Math.max(1, Math.round(props.barCount));
  return Math.max(8, Math.round(width / FIT_UNIT));
}

/**
 * Resample non-DC FFT bins to logarithmic frequency bands. Bands are kept
 * fractional: at the bass end a band is narrower than a single bin, so we
 * interpolate between the two bins it spans instead of rounding both edges to
 * the same bin (which painted a long flat plateau on the left). Wider bands
 * still take their peak, which keeps the treble lively. Bin zero is DC and is
 * removed by the native FFT preprocessing, so the scale starts at bin one.
 */
function sampleBars(total: number): number[] {
  const source = spectrumData.value?.magnitudes ?? [];
  if (source.length === 0) return Array.from({ length: total }, () => 0.06);
  const firstBin = 1;
  const lastBin = source.length - 1;
  const ratio = Math.pow(lastBin / firstBin, 1 / Math.max(1, total));
  return Array.from({ length: total }, (_, index) => {
    const start = firstBin * Math.pow(ratio, index);
    const end = firstBin * Math.pow(ratio, index + 1);
    let linear: number;
    if (end - start < 1) {
      const lower = Math.min(lastBin, Math.max(firstBin, Math.floor(start)));
      const upper = Math.min(lastBin, lower + 1);
      const mix = Math.min(1, Math.max(0, start - lower));
      linear = source[lower] + (source[upper] - source[lower]) * mix;
    } else {
      const lo = Math.min(lastBin, Math.max(firstBin, Math.floor(start)));
      const hi = Math.min(lastBin, Math.max(lo, Math.ceil(end) - 1));
      linear = Math.max(...source.slice(lo, hi + 1), 0);
    }
    const db = 20 * Math.log10(Math.max(linear, 0.00001));
    return Math.max(0.06, Math.min(1, (db + 60) / 60));
  });
}

function paint(total: number, targets: number[], width: number, height: number, ease: number) {
  if (!ctx) return;
  const pitch = props.fit ? width / total : BAR_WIDTH + BAR_GAP;
  const barWidth = Math.min(pitch, Math.max(0.5, pitch - (props.fit ? FIT_GAP : BAR_GAP)));
  const startX = props.fit ? 0 : (width - (total * pitch - BAR_GAP)) / 2;

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = color;
  ctx.globalAlpha = BAR_OPACITY;

  const rounded = typeof ctx.roundRect === "function";
  if (rounded) ctx.beginPath();
  for (let index = 0; index < total; index += 1) {
    const value = current[index] + (targets[index] - current[index]) * ease;
    current[index] = value;
    const barHeight = Math.max(MIN_BAR_HEIGHT, value * height);
    const x = startX + index * pitch;
    const y = height - barHeight;
    if (rounded) ctx.roundRect(x, y, barWidth, barHeight, Math.min(barWidth, barHeight) / 2);
    else ctx.fillRect(x, y, barWidth, barHeight);
  }
  if (rounded) ctx.fill();
}

function loop(now: number) {
  frame = requestAnimationFrame(loop);
  const el = canvas.value;
  if (!el || !ctx) return;
  const width = el.clientWidth;
  const height = el.clientHeight;
  if (width <= 0 || height <= 0) return;
  const total = barCountFor(width);
  const targets = sampleBars(total);
  if (current.length !== total) current = targets.slice();
  const dt = lastTime ? Math.min(now - lastTime, 64) : SMOOTH_MS;
  lastTime = now;
  paint(total, targets, width, height, Math.min(1, dt / SMOOTH_MS));
}

function observe() {
  resize();
  if (typeof ResizeObserver === "undefined" || !canvas.value) return;
  observer?.disconnect();
  observer = new ResizeObserver(resize);
  observer.observe(canvas.value);
}

onMounted(() => {
  observe();
  frame = requestAnimationFrame(loop);
  if (typeof MutationObserver !== "undefined") {
    themeObserver = new MutationObserver(loadColor);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style", "data-theme"] });
  }
});

onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  observer?.disconnect();
  observer = null;
  themeObserver?.disconnect();
  themeObserver = null;
});

watch(showWaveform, async (visible) => {
  await nextTick();
  if (visible) {
    observe();
    return;
  }
  observer?.disconnect();
  observer = null;
  current = [];
  lastTime = 0;
});
</script>

<template>
  <canvas
    v-if="showWaveform"
    ref="canvas"
    class="realtime-waveform block w-full"
    :style="{ height: `${DEFAULT_HEIGHT * waveformScale}px` }"
    aria-label="Realtime frequency spectrum"
    role="img"
  ></canvas>
</template>
