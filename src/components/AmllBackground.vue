<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { BackgroundRender, MeshGradientRenderer } from "@applemusic-like-lyrics/core";

const props = withDefaults(defineProps<{ album?: string; fps?: number; renderScale?: number }>(), {
  fps: 30,
  renderScale: 0.5,
});
/** Emitted once the first fully-rendered frame is on screen. */
const emit = defineEmits<{ ready: [] }>();

const host = ref<HTMLDivElement | null>(null);

/**
 * Keep the renderer outside Vue's reactivity on purpose. Wrapping the AMLL
 * renderer instance in a reactive `ref` makes its internal mutations trigger
 * their own effects and Vue bails out with "Maximum recursive updates exceeded",
 * which aborts the update scheduler.
 */
let bg: BackgroundRender<MeshGradientRenderer> | null = null;
let canvas: HTMLCanvasElement | null = null;
let isReady = false;
let disposed = false;
let readyTimer = 0;

/** Runtime-only internals of `MeshGradientRenderer` (typings mark them private). */
type MeshState = { alpha: number };
type MeshRendererFull = { meshStates?: MeshState[] };

function signalReady() {
  if (isReady || disposed) return;
  isReady = true;
  emit("ready");
}

function signalReadyAfterFrame() {
  requestAnimationFrame(() => requestAnimationFrame(signalReady));
}

/**
 * A new album mesh cross-fades in over ~500ms (alpha 0 -> 1.1). Snap it to full
 * opacity so the first painted frame is already complete and the slide-in never
 * shows a half-rendered gradient.
 */
function forceMeshFull(): boolean {
  if (!bg) return false;
  const renderer = bg.getRenderer() as unknown as MeshRendererFull;
  const states = renderer.meshStates;
  const latest = states?.[states.length - 1];
  if (!latest) return false;
  latest.alpha = 1.1;
  return true;
}

/** Poll until the canvas is sized and the album mesh is drawn, then report ready. */
function waitUntilRendered() {
  const deadline = performance.now() + 1200;
  const tick = () => {
    if (disposed) return;
    const sized = !!canvas && canvas.width > 0 && canvas.height > 0;
    const drawn = props.album ? forceMeshFull() : true;
    if (sized && drawn) { signalReadyAfterFrame(); return; }
    if (performance.now() > deadline) { signalReadyAfterFrame(); return; }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

onMounted(() => {
  const hostEl = host.value;
  if (!hostEl) return;
  const instance = BackgroundRender.new(MeshGradientRenderer);
  canvas = instance.getElement();
  canvas.style.width = "100%";
  canvas.style.height = "100%";
  hostEl.appendChild(canvas);
  instance.setFPS(props.fps);
  instance.setRenderScale(props.renderScale);
  instance.setHasLyric(true);
  // Keep animating even while paused so the mesh is there the moment it mounts.
  instance.resume();
  bg = instance;
  if (props.album) void instance.setAlbum(props.album);
  waitUntilRendered();
  // Safety net: never leave the page waiting on a slow/failed album load.
  readyTimer = window.setTimeout(signalReady, 1800);
});

watch(
  () => props.album,
  (album) => { if (bg && album) void bg.setAlbum(album); },
);

onBeforeUnmount(() => {
  disposed = true;
  window.clearTimeout(readyTimer);
  bg?.dispose();
  bg = null;
  canvas = null;
});
</script>

<template>
  <div ref="host" class="h-full w-full"></div>
</template>
