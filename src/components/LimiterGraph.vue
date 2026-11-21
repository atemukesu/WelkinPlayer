<!--
  Copyright 2026 Atemukesu
  SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  limiterEnabled,
  limiterMeter,
  limiterThreshold,
  listenLimiterMeter,
} from "../lib/limiter";

/**
 * Live limiter graph, in the spirit of FL Studio's limiter.
 *
 * The engine publishes one peak pair per 50 ms tick. The waveform plot draws
 * the pre-limiter and post-limiter upper envelopes as plain lines on a
 * logarithmic (dB) axis, so a gain reduction is visible as the accent output
 * curve flattening at the threshold while the dim input curve stays above it.
 *
 * A separate strip on the right, exactly as tall as the plot, is the meter:
 * the accent bar rises with the current output level, while the grey bar falls
 * from the top with the gain reduction the limiter is applying.
 */

/** Visible history, in meter ticks (50 ms each → 6 s). */
const HISTORY = 120;
/** Vertical dB range: a little headroom above 0, floor below the quietest peaks. */
const MAX_DB = 6;
const MIN_DB = -24;
/** Evenly spaced 6 dB grid on a logarithmic (dB) axis. */
const GRID_DB = [0, -6, -12, -18, -24] as const;
/** Pixels kept clear above the top and below the bottom of the plot. */
const PAD_Y = 8;
/** Gain reduction in dB that fills the strip completely. */
const METER_FULL_DB = 12;

const canvas = ref<HTMLCanvasElement | null>(null);
const meterCanvas = ref<HTMLCanvasElement | null>(null);
const { t } = useI18n();
let ctx: CanvasRenderingContext2D | null = null;
let meterCtx: CanvasRenderingContext2D | null = null;
let observer: ResizeObserver | null = null;
let themeObserver: MutationObserver | null = null;
let stopMeter: (() => void) | null = null;
let frame = 0;

const input = new Array<number>(HISTORY).fill(MIN_DB);
const output = new Array<number>(HISTORY).fill(MIN_DB);

let colors = {
  accent: "#f0a500",
  muted: "#9aa1ab",
  dim: "#616873",
  line: "rgba(128,128,128,0.22)",
  fg: "#f3f4f6",
};
let lastW = 0;
let lastH = 0;
let lastDpr = 0;
let lastMeterW = 0;
let lastMeterH = 0;
let lastMeterDpr = 0;
/** Smoothed output level for the meter, in dB. */
let levelDb = MIN_DB;
/** Smoothed gain reduction for the meter, in positive dB. */
let meterDb = 0;

function readColors() {
  const style = getComputedStyle(document.documentElement);
  const read = (name: string) => style.getPropertyValue(name).trim();
  colors = {
    accent: read("--accent") || colors.accent,
    muted: read("--muted") || colors.muted,
    dim: read("--dim") || colors.dim,
    line: read("--line") || colors.line,
    fg: read("--fg") || colors.fg,
  };
}

function resize() {
  const dpr = window.devicePixelRatio || 1;
  const el = canvas.value;
  if (el) {
    const width = el.clientWidth;
    const height = el.clientHeight;
    if (width > 0 && height > 0 && (width !== lastW || height !== lastH || dpr !== lastDpr)) {
      lastW = width;
      lastH = height;
      lastDpr = dpr;
      el.width = Math.round(width * dpr);
      el.height = Math.round(height * dpr);
      ctx = el.getContext("2d");
      ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    }
  }
  const meter = meterCanvas.value;
  if (meter) {
    const width = meter.clientWidth;
    const height = meter.clientHeight;
    if (
      width > 0 &&
      height > 0 &&
      (width !== lastMeterW || height !== lastMeterH || dpr !== lastMeterDpr)
    ) {
      lastMeterW = width;
      lastMeterH = height;
      lastMeterDpr = dpr;
      meter.width = Math.round(width * dpr);
      meter.height = Math.round(height * dpr);
      meterCtx = meter.getContext("2d");
      meterCtx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    }
  }
  readColors();
}

/** Convert a linear peak to dBFS, clamped to the visible range. */
function toDb(amplitude: number): number {
  if (!(amplitude > 0)) return MIN_DB;
  return Math.max(MIN_DB, Math.min(MAX_DB, 20 * Math.log10(amplitude)));
}

/** Push one meter tick, keeping a fixed-length scrolling history. */
function pushTick(inputPeak: number, outputPeak: number) {
  input.push(toDb(inputPeak));
  output.push(toDb(outputPeak));
  if (input.length > HISTORY) input.splice(0, input.length - HISTORY);
  if (output.length > HISTORY) output.splice(0, output.length - HISTORY);
}

/** Map a dB value onto the plot; higher dB is nearer the top. */
function dbY(db: number, top: number, plot: number): number {
  const clamped = Math.max(MIN_DB, Math.min(MAX_DB, db));
  return top + ((MAX_DB - clamped) / (MAX_DB - MIN_DB)) * plot;
}

/** Upper envelope only: one open polyline, no mirrored lower half. */
function trace(values: number[], top: number, plot: number, step: number): Path2D {
  const path = new Path2D();
  const count = values.length;
  const startX = -Math.max(0, HISTORY - count) * step;
  for (let index = 0; index < count; index += 1) {
    const x = startX + index * step;
    const y = dbY(values[index], top, plot);
    if (index === 0) path.moveTo(x, y);
    else path.lineTo(x, y);
  }
  return path;
}

function paint(width: number, height: number) {
  if (!ctx) return;
  const top = PAD_Y;
  const plot = Math.max(1, height - PAD_Y * 2);

  ctx.clearRect(0, 0, width, height);

  // Horizontal dB grid: evenly spaced because the axis is logarithmic.
  ctx.font = "9px ui-monospace, monospace";
  ctx.textBaseline = "middle";
  ctx.textAlign = "left";
  ctx.lineWidth = 1;
  for (const db of GRID_DB) {
    const y = dbY(db, top, plot);
    ctx.strokeStyle = colors.line;
    ctx.globalAlpha = db === 0 ? 0.8 : 0.45;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.fillStyle = colors.dim;
    ctx.fillText(`${db}`, 6, Math.max(7, y - 5));
  }

  // Upper envelopes only, as plain lines: no fill, no glow, no mirroring.
  const step = width / Math.max(1, HISTORY - 1);
  ctx.lineJoin = "round";
  ctx.lineCap = "round";

  // Input peaks, dim and thin, behind the output.
  ctx.strokeStyle = colors.muted;
  ctx.globalAlpha = 0.55;
  ctx.lineWidth = 1.25;
  ctx.stroke(trace(input, top, plot, step));
  ctx.globalAlpha = 1;

  // Output peaks, in front and slightly brighter.
  ctx.strokeStyle = colors.accent;
  ctx.lineWidth = 1.75;
  ctx.stroke(trace(output, top, plot, step));

  // Threshold / ceiling: a single quiet reference line.
  const thresholdY = dbY(limiterThreshold.value, top, plot);
  ctx.strokeStyle = colors.fg;
  ctx.globalAlpha = 0.4;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, thresholdY);
  ctx.lineTo(width, thresholdY);
  ctx.stroke();
  ctx.globalAlpha = 0.65;
  ctx.fillStyle = colors.fg;
  ctx.textAlign = "right";
  ctx.fillText(
    `${t("soundSettings.limiter.graph.threshold")} ${limiterThreshold.value.toFixed(1)} dB`,
    width - 6,
    Math.max(8, thresholdY - 7),
  );
  ctx.globalAlpha = 1;
  ctx.textAlign = "left";
}

/**
 * The right-hand strip: output level (accent, rising from the bottom) and gain
 * reduction (grey, falling from the top). Two narrow bars keep both legible at
 * the width of a single meter.
 */
function paintMeter(width: number, height: number) {
  if (!meterCtx) return;
  const top = PAD_Y;
  const plot = Math.max(1, height - PAD_Y * 2);
  const gap = 3;
  const barW = Math.max(3, Math.floor((width - gap - 6) / 2));
  const total = barW * 2 + gap;
  const leftX = Math.floor((width - total) / 2);
  const rightX = leftX + barW + gap;

  meterCtx.clearRect(0, 0, width, height);

  // Tracks.
  meterCtx.fillStyle = colors.line;
  meterCtx.fillRect(leftX, top, barW, plot);
  meterCtx.fillRect(rightX, top, barW, plot);

  // Output level: the current peak, rising from the bottom.
  levelDb += (toDb(limiterMeter.outputPeak) - levelDb) * 0.35;
  const level = Math.max(0, Math.min(1, (levelDb - MIN_DB) / (MAX_DB - MIN_DB)));
  const levelHeight = level * plot;
  if (levelHeight > 0.5) {
    meterCtx.fillStyle = colors.accent;
    meterCtx.fillRect(leftX, top + plot - levelHeight, barW, levelHeight);
  }

  // Gain reduction: how far the limiter is pulling the level down.
  meterDb += (Math.max(0, -limiterMeter.gainReductionDb) - meterDb) * 0.18;
  const reduction = Math.min(1, meterDb / METER_FULL_DB);
  if (reduction > 0.002) {
    meterCtx.fillStyle = colors.muted;
    meterCtx.fillRect(rightX, top, barW, Math.max(1, reduction * plot));
  }
}

function loop() {
  frame = requestAnimationFrame(loop);
  const el = canvas.value;
  if (el && ctx) {
    const width = el.clientWidth;
    const height = el.clientHeight;
    if (width > 0 && height > 0) paint(width, height);
  }
  const meter = meterCanvas.value;
  if (meter && meterCtx) {
    const width = meter.clientWidth;
    const height = meter.clientHeight;
    if (width > 0 && height > 0) paintMeter(width, height);
  }
}

onMounted(() => {
  readColors();
  resize();
  if (typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(resize);
    if (canvas.value) observer.observe(canvas.value);
    if (meterCanvas.value) observer.observe(meterCanvas.value);
  }
  if (typeof MutationObserver !== "undefined") {
    themeObserver = new MutationObserver(() => {
      readColors();
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style", "data-theme", "data-accent"],
    });
  }
  stopMeter = listenLimiterMeter((meter) => pushTick(meter.inputPeak, meter.outputPeak));
  frame = requestAnimationFrame(loop);
});

onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  observer?.disconnect();
  observer = null;
  themeObserver?.disconnect();
  themeObserver = null;
  stopMeter?.();
  stopMeter = null;
});
</script>

<template>
  <div class="border border-line bg-bg/50">
    <div class="flex items-stretch">
      <div class="relative min-w-0 flex-1">
        <canvas
          ref="canvas"
          class="block h-[168px] w-full"
          role="img"
          aria-label="Limiter input and output waveform"
        ></canvas>

        <div
          class="pointer-events-none absolute inset-x-2 top-1 flex items-center text-[9px] font-semibold uppercase tracking-[0.14em]"
        >
          <div class="flex items-center gap-3">
            <span class="flex items-center gap-1 text-dim">
              <span class="h-[2px] w-3" style="background: var(--muted)"></span>
              {{ t("soundSettings.limiter.graph.in") }}
            </span>
            <span class="flex items-center gap-1 text-accent">
              <span class="h-[2px] w-3 bg-accent"></span>
              {{ t("soundSettings.limiter.graph.out") }}
            </span>
          </div>
        </div>

        <div
          v-if="!limiterEnabled"
          class="pointer-events-none absolute bottom-1.5 right-2 border border-line bg-surface/80 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.18em] text-dim"
        >
          {{ t("soundSettings.limiter.graph.bypass") }}
        </div>
      </div>

      <div class="w-5 shrink-0 border-l border-line">
        <canvas
          ref="meterCanvas"
          class="block h-[168px] w-full"
          role="img"
          aria-label="Output level and gain reduction"
        ></canvas>
      </div>
    </div>
  </div>
</template>
