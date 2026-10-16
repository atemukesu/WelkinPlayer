<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "../api";

/**
 * Full-screen spectrum visualiser.
 *
 * Analysis lives in the playback engine (`playback::spectrum`) because decoded
 * audio never reaches the webview on this build — the Rust engine hands it
 * straight to the device. Frames arrive as small `audio:spectrum` events; this
 * component only paints them, which is what keeps it usable on software-rendered
 * kiosk builds: no Web Audio, no CORS on the stream, and a fill rate we control.
 *
 * The layer is deliberately opaque: it is a screen of its own, not an overlay on
 * the library UI. Moving the mouse reveals the exit chip, which fades away again
 * after a few idle seconds.
 */
const props = withDefaults(
  defineProps<{
    /** Note: not `style` — that name collides with Vue's CSS style binding. */
    variant?: "mirror" | "bars" | "radial";
    color?: string;
    bands?: number;
    fps?: number;
    renderScale?: number;
  }>(),
  { variant: "mirror", color: "#ff6ad5", bands: 72, fps: 30, renderScale: 0.6 },
);

const emit = defineEmits<{ exit: [] }>();

interface SpectrumFrame {
  seq: number;
  bands: number[];
}

const CHROME_TIMEOUT_MS = 2600;
const FRAME_HOLD_MS = 250;
const PEAK_FALL = 0.014;
/** Floating triangles, matching the look of the reference screen. */
const PARTICLES = 14;

const host = ref<HTMLDivElement | null>(null);
const chromeVisible = ref(true);

let canvas: HTMLCanvasElement | null = null;
let context: CanvasRenderingContext2D | null = null;
let unlisten: UnlistenFn | null = null;
let observer: ResizeObserver | null = null;
let raf = 0;
let chromeTimer = 0;
let disposed = false;
let width = 0;
let height = 0;

/** Latest frame from the engine, plus the decaying peak caps drawn on top. */
let levels = new Float32Array(props.bands);
let peaks = new Float32Array(props.bands);
let lastFrameAt = 0;
let drawnFrames = 0;
let measureFrom = 0;
let lastPaint = 0;

/** Slow-drifting triangles, seeded once so the field is stable per session. */
const particles = Array.from({ length: PARTICLES }, (_, index) => ({
  x: (index * 137.5) % 1000 / 1000,
  y: (index * 71.3) % 1000 / 1000,
  size: 0.05 + ((index * 37) % 100) / 900,
  spin: ((index * 53) % 100) / 100 * Math.PI,
  speed: 0.000012 + ((index * 17) % 50) / 4_000_000,
  alpha: 0.05 + ((index * 29) % 60) / 900,
}));

function resize() {
  if (!canvas) return;
  const cssWidth = canvas.clientWidth || window.innerWidth;
  const cssHeight = canvas.clientHeight || window.innerHeight;
  if (cssWidth <= 0 || cssHeight <= 0) return;
  // The layer is full-screen and software-rendered on kiosk builds, so fill a
  // fraction of the pixels and let the compositor scale the bars up.
  const scale = Math.min(1, Math.max(0.25, props.renderScale)) * Math.min(window.devicePixelRatio || 1, 2);
  width = cssWidth;
  height = cssHeight;
  canvas.width = Math.max(1, Math.floor(cssWidth * scale));
  canvas.height = Math.max(1, Math.floor(cssHeight * scale));
  context = canvas.getContext("2d");
  if (context) context.setTransform(scale, 0, 0, scale, 0, 0);
}

/** `#rrggbb` (or `#rgb`) to `rgba(...)`. */
function withAlpha(value: string, alpha: number): string {
  const hex = value.replace("#", "");
  const full = hex.length === 3 ? hex.split("").map((c) => c + c).join("") : hex;
  const parsed = Number.parseInt(full.slice(0, 6) || "ff6ad5", 16);
  const r = (parsed >> 16) & 255;
  const g = (parsed >> 8) & 255;
  const b = parsed & 255;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Opaque black base with a soft glow of the track colour behind the bars. */
function paintBackdrop(ctx: CanvasRenderingContext2D, time: number) {
  ctx.fillStyle = "#000000";
  ctx.fillRect(0, 0, width, height);

  const glow = ctx.createRadialGradient(width / 2, height / 2, 0, width / 2, height / 2, Math.max(width, height) * 0.62);
  glow.addColorStop(0, withAlpha(props.color, 0.34));
  glow.addColorStop(0.45, withAlpha(props.color, 0.12));
  glow.addColorStop(1, "rgba(0, 0, 0, 0)");
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, width, height);

  ctx.fillStyle = withAlpha(props.color, 1);
  for (const particle of particles) {
    const x = ((particle.x + time * particle.speed) % 1) * width;
    const y = particle.y * height;
    const size = particle.size * Math.min(width, height) * 0.5;
    ctx.save();
    ctx.translate(x, y);
    ctx.rotate(particle.spin + time * 0.00004);
    ctx.globalAlpha = particle.alpha;
    ctx.beginPath();
    ctx.moveTo(0, -size);
    ctx.lineTo(size * 0.9, size * 0.7);
    ctx.lineTo(-size * 0.9, size * 0.7);
    ctx.closePath();
    ctx.fill();
    ctx.restore();
  }
  ctx.globalAlpha = 1;
}

/** Mirrored bars around the centre line, with a wide glow pass underneath. */
function paintMirror(ctx: CanvasRenderingContext2D, slot: number, barWidth: number, gap: number, half: number) {
  const centre = height / 2;
  const white = "rgba(255, 255, 255, 0.95)";

  // Glow pass first: wider, translucent, so the bright pass reads as neon.
  ctx.fillStyle = withAlpha(props.color, 0.5);
  for (let i = 0; i < levels.length; i += 1) {
    const barHeight = Math.max(2, Math.pow(Math.min(1, levels[i]), 0.8) * half);
    const x = i * slot + gap;
    ctx.fillRect(x - barWidth * 0.35, centre - barHeight, barWidth * 1.7, barHeight * 2);
  }

  ctx.fillStyle = white;
  for (let i = 0; i < levels.length; i += 1) {
    const barHeight = Math.max(1.5, Math.pow(Math.min(1, levels[i]), 0.8) * half);
    const x = i * slot + gap;
    ctx.fillRect(x, centre - barHeight, barWidth, barHeight * 2);
  }

  // Centre rail, drawn over the bars like the reference screen.
  ctx.fillStyle = "rgba(255, 255, 255, 0.88)";
  ctx.fillRect(0, centre - 1, width, 2);

  // Vinyl disc sitting on the rail.
  const radius = Math.min(height * 0.11, width * 0.07);
  ctx.fillStyle = withAlpha(props.color, 0.55);
  ctx.beginPath();
  ctx.arc(width * 0.5, centre, radius, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
  ctx.beginPath();
  ctx.arc(width * 0.5, centre, radius * 0.22, 0, Math.PI * 2);
  ctx.fill();
}

/** Classic bottom-anchored bars. */
function paintBars(ctx: CanvasRenderingContext2D, slot: number, barWidth: number, gap: number) {
  ctx.fillStyle = withAlpha(props.color, 0.45);
  for (let i = 0; i < levels.length; i += 1) {
    const barHeight = Math.max(2, Math.pow(Math.min(1, levels[i]), 0.85) * height * 0.94);
    ctx.fillRect(i * slot + gap - barWidth * 0.3, height - barHeight, barWidth * 1.6, barHeight);
  }
  ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
  for (let i = 0; i < levels.length; i += 1) {
    const barHeight = Math.max(1.5, Math.pow(Math.min(1, levels[i]), 0.85) * height * 0.94);
    ctx.fillRect(i * slot + gap, height - barHeight, barWidth, barHeight);
  }
  ctx.fillStyle = "rgba(255, 255, 255, 0.9)";
  for (let i = 0; i < levels.length; i += 1) {
    const y = height - Math.min(1, peaks[i]) * height * 0.94 - 4;
    if (y > 6) ctx.fillRect(i * slot + gap, y, barWidth, 3);
  }
}

/** Radial spokes around the vinyl disc. */
function paintRadial(ctx: CanvasRenderingContext2D, time: number) {
  const centreX = width / 2;
  const centreY = height / 2;
  const inner = Math.min(width, height) * 0.14;
  const outer = Math.min(width, height) * 0.44;
  const count = levels.length;

  ctx.save();
  ctx.translate(centreX, centreY);
  for (let i = 0; i < count; i += 1) {
    const angle = (i / count) * Math.PI * 2 - Math.PI / 2;
    const reach = inner + Math.pow(Math.min(1, levels[i]), 0.85) * (outer - inner);
    ctx.save();
    ctx.rotate(angle + time * 0.00003);
    ctx.fillStyle = withAlpha(props.color, 0.5);
    ctx.fillRect(-2.4, -reach, 4.8, reach - inner);
    ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
    ctx.fillRect(-1, -reach, 2, reach - inner);
    ctx.restore();
  }
  ctx.restore();

  const radius = inner * 0.86;
  ctx.fillStyle = withAlpha(props.color, 0.5);
  ctx.beginPath();
  ctx.arc(centreX, centreY, radius, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "rgba(255, 255, 255, 0.95)";
  ctx.beginPath();
  ctx.arc(centreX, centreY, radius * 0.2, 0, Math.PI * 2);
  ctx.fill();
}

function paint(time: number) {
  const ctx = context;
  if (!ctx || width <= 0 || height <= 0) return;

  paintBackdrop(ctx, time);

  const count = levels.length;
  if (!count) return;
  const slot = width / count;
  const barWidth = Math.max(1.5, slot * 0.34);
  const gap = (slot - barWidth) / 2;

  if (props.variant === "bars") {
    paintBars(ctx, slot, barWidth, gap);
  } else if (props.variant === "radial") {
    paintRadial(ctx, time);
  } else {
    paintMirror(ctx, slot, barWidth, gap, height / 2 - 8);
  }
}

function tick(now: number) {
  if (disposed) return;
  raf = requestAnimationFrame(tick);

  if (!width || !height) resize();

  const live = now - lastFrameAt < FRAME_HOLD_MS;
  const interval = 1000 / Math.max(5, props.fps);
  if (now - lastPaint < interval) return;
  lastPaint = now;

  for (let i = 0; i < levels.length; i += 1) {
    if (!live) levels[i] *= 0.92; // settle instead of freezing when playback stops
    peaks[i] = Math.max(levels[i], peaks[i] - PEAK_FALL);
  }

  paint(now);
  drawnFrames += 1;

  if (!measureFrom) measureFrom = now;
  if (now - measureFrom >= 2000) {
    const fps = Math.round((drawnFrames * 1000) / (now - measureFrom));
    const box = host.value?.getBoundingClientRect();
    void invoke("log_message", {
      level: "info",
      message: `[visualizer] ${props.variant} ${fps} fps, ${levels.length} bands, canvas ${width}x${height}, layer ${Math.round(box?.width ?? -1)}x${Math.round(box?.height ?? -1)}`,
    }).catch(() => {});
    drawnFrames = 0;
    measureFrom = now;
  }
}

/** Reveal the exit chip, then fade it out again while the mouse stays idle. */
function showChrome() {
  chromeVisible.value = true;
  window.clearTimeout(chromeTimer);
  chromeTimer = window.setTimeout(() => {
    chromeVisible.value = false;
  }, CHROME_TIMEOUT_MS);
}

async function configure(on: boolean) {
  try {
    await invoke("audio_spectrum_config", { enabled: on, bands: props.bands, fps: props.fps });
  } catch (error) {
    console.warn("[welkin] spectrum config failed", error);
  }
}

async function subscribe() {
  unlisten = await listen<SpectrumFrame>("audio:spectrum", (event) => {
    const bands = event.payload?.bands ?? [];
    if (!bands.length) return;
    if (levels.length !== bands.length) {
      levels = new Float32Array(bands.length);
      peaks = new Float32Array(bands.length);
    }
    for (let i = 0; i < bands.length; i += 1) levels[i] = bands[i] / 255;
    lastFrameAt = performance.now();
  });
}

/**
 * Escape leaves the visualiser, not the application: the Batocera launcher maps
 * Escape to "quit" (see `WELKIN_ESC_QUITS`), so this layer has to consume the key
 * first, in the capture phase, or the whole player would exit instead.
 */
function onKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  event.preventDefault();
  event.stopPropagation();
  emit("exit");
}

onMounted(async () => {
  canvas = document.createElement("canvas");
  canvas.style.width = "100%";
  canvas.style.height = "100%";
  canvas.style.display = "block";
  host.value?.appendChild(canvas);
  resize();
  observer = new ResizeObserver(() => resize());
  observer.observe(host.value as HTMLDivElement);
  window.addEventListener("resize", resize);
  window.addEventListener("mousemove", showChrome);
  window.addEventListener("keydown", onKeydown, true);
  showChrome();
  await subscribe();
  await configure(true);
  raf = requestAnimationFrame(tick);
});

onBeforeUnmount(() => {
  disposed = true;
  cancelAnimationFrame(raf);
  window.clearTimeout(chromeTimer);
  observer?.disconnect();
  observer = null;
  window.removeEventListener("resize", resize);
  window.removeEventListener("mousemove", showChrome);
  window.removeEventListener("keydown", onKeydown, true);
  unlisten?.();
  unlisten = null;
  void configure(false);
  canvas?.remove();
  canvas = null;
  context = null;
});

defineExpose({ showChrome });
</script>

<template>
  <!-- A screen of its own: opaque black, above every view. Inline styles on
       purpose — this layer must not depend on utility classes being generated. -->
  <div
    :style="{
      position: 'fixed',
      inset: '0',
      zIndex: 70,
      background: '#000',
      cursor: chromeVisible ? 'default' : 'none',
    }"
    @mousemove="showChrome"
  >
    <div ref="host" style="width: 100%; height: 100%"></div>
    <Transition name="fade">
      <div
        v-show="chromeVisible"
        :style="{
          position: 'absolute',
          left: '0',
          right: '0',
          bottom: '0',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          gap: '12px',
          paddingBottom: '40px',
        }"
      >
        <p style="font-size: 12px; font-weight: 600; letter-spacing: 0.25em; text-transform: uppercase; color: rgba(255,255,255,0.45)">
          {{ $t("visualizer.exitHint") }}
        </p>
        <button
          type="button"
          style="display: flex; height: 44px; align-items: center; gap: 8px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.25); background: rgba(255,255,255,0.1); padding: 0 24px; font-size: 14px; font-weight: 600; color: rgba(255,255,255,0.92)"
          @click="emit('exit')"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
            <path d="m16 17 5-5-5-5" />
            <path d="M21 12H9" />
          </svg>
          {{ $t("visualizer.exit") }}
        </button>
      </div>
    </Transition>
  </div>
</template>
