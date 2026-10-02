<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Check, RotateCcw, X } from "@lucide/vue";

const props = defineProps<{ src: string }>();
const emit = defineEmits<{ confirm: [value: string]; cancel: [] }>();
const { t } = useI18n();

/** Square viewport size in CSS pixels; must stay in sync with the template. */
const SIZE = 256;
const natural = ref({ w: 0, h: 0 });
const zoom = ref(1);
const dx = ref(0);
const dy = ref(0);

const fit = computed(() => natural.value.w > 0 ? Math.max(SIZE / natural.value.w, SIZE / natural.value.h) : 1);
const loaded = computed(() => natural.value.w > 0);
const displayW = computed(() => natural.value.w * fit.value * zoom.value);
const displayH = computed(() => natural.value.h * fit.value * zoom.value);

function clamp() {
  const maxX = Math.max(0, (displayW.value - SIZE) / 2);
  const maxY = Math.max(0, (displayH.value - SIZE) / 2);
  dx.value = Math.min(maxX, Math.max(-maxX, dx.value));
  dy.value = Math.min(maxY, Math.max(-maxY, dy.value));
}

let drag: { x: number; y: number; dx: number; dy: number } | null = null;
function onDown(event: PointerEvent) {
  drag = { x: event.clientX, y: event.clientY, dx: dx.value, dy: dy.value };
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}
function onMove(event: PointerEvent) {
  if (!drag) return;
  dx.value = drag.dx + (event.clientX - drag.x);
  dy.value = drag.dy + (event.clientY - drag.y);
  clamp();
}
function onUp(event: PointerEvent) {
  drag = null;
  try { (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId); } catch { /* pointer already released */ }
}
function reset() { zoom.value = 1; dx.value = 0; dy.value = 0; }

onMounted(() => {
  const image = new Image();
  image.onload = () => { natural.value = { w: image.naturalWidth, h: image.naturalHeight }; };
  image.src = props.src;
});

async function confirm() {
  const image = new Image();
  await new Promise<void>((resolve, reject) => { image.onload = () => resolve(); image.onerror = () => reject(new Error("image")); image.src = props.src; });
  const output = 512;
  const canvas = document.createElement("canvas");
  canvas.width = output;
  canvas.height = output;
  const context = canvas.getContext("2d");
  if (!context) return;
  const dw = displayW.value;
  const dh = displayH.value;
  const left = (SIZE - dw) / 2 + dx.value;
  const top = (SIZE - dh) / 2 + dy.value;
  const sx = ((0 - left) / dw) * natural.value.w;
  const sy = ((0 - top) / dh) * natural.value.h;
  const sw = (SIZE / dw) * natural.value.w;
  const sh = (SIZE / dh) * natural.value.h;
  context.imageSmoothingQuality = "high";
  context.drawImage(image, sx, sy, sw, sh, 0, 0, output, output);
  emit("confirm", canvas.toDataURL("image/jpeg", 0.85));
}
</script>

<template>
  <div class="fixed inset-0 z-[80] grid place-items-center bg-black/70 p-4" @pointerdown.self="emit('cancel')">
    <div class="ak-frame w-full max-w-sm border border-line bg-surface p-5">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-bold uppercase tracking-[0.2em]">{{ t("playlistEditor.cropTitle") }}</h3>
        <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" @click="emit('cancel')"><X :size="16" /></button>
      </div>
      <p class="mt-2 text-[12px] text-dim">{{ t("playlistEditor.cropHint") }}</p>

      <div class="mt-4 flex justify-center">
        <div
          class="relative touch-none select-none overflow-hidden border border-line bg-black"
          :style="{ width: `${SIZE}px`, height: `${SIZE}px` }"
          @pointerdown="onDown"
          @pointermove="onMove"
          @pointerup="onUp"
          @pointercancel="onUp"
        >
          <img
            :src="src"
            alt=""
            draggable="false"
            class="absolute left-1/2 top-1/2 max-w-none"
            :style="{ width: `${displayW}px`, height: `${displayH}px`, transform: `translate(calc(-50% + ${dx}px), calc(-50% + ${dy}px))` }"
          />
          <span class="pointer-events-none absolute inset-0 ring-1 ring-inset ring-white/30"></span>
        </div>
      </div>

      <div class="mt-4 flex items-center gap-3">
        <span class="text-[11px] font-semibold uppercase tracking-[0.2em] text-dim">{{ t("playlistEditor.zoom") }}</span>
        <input v-model.number="zoom" type="range" min="1" max="4" step="0.01" class="ak-slider flex-1" :style="{ '--fill': `${((zoom - 1) / 3) * 100}%` }" @input="clamp" />
        <button type="button" class="grid h-8 w-8 place-items-center text-dim hover:text-fg" :title="t('playlistEditor.reset')" @click="reset"><RotateCcw :size="15" /></button>
      </div>

      <div class="mt-5 flex justify-end gap-3">
        <button type="button" class="ak-clip-tr h-10 border border-line px-4 text-[13px] font-semibold" @click="emit('cancel')">{{ t("playlistEditor.cancel") }}</button>
        <button type="button" class="ak-clip-tr flex h-10 items-center gap-2 bg-accent px-5 text-[13px] font-bold text-accent-fg disabled:opacity-50" :disabled="!loaded" @click="confirm"><Check :size="15" />{{ t("playlistEditor.apply") }}</button>
      </div>
    </div>
  </div>
</template>
