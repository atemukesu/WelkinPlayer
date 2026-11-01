<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { LetterIndexEntry } from "../lib/sort";

const props = defineProps<{ entries: LetterIndexEntry[] }>();
const emit = defineEmits<{ select: [entry: LetterIndexEntry] }>();
const { t } = useI18n();

const rail = ref<HTMLElement | null>(null);
/** Letter currently under the pointer, shown in the floating bubble. */
const active = ref<string | null>(null);
const dragging = ref(false);
let hideTimer = 0;
/** Dedupes a tap, whose pointerdown and click both resolve to the same label. */
let lastPick: { label: string; at: number } | null = null;

const activeTop = computed(() => {
  const index = props.entries.findIndex((entry) => entry.label === active.value);
  if (index < 0) return "50%";
  return `${((index + 0.5) / props.entries.length) * 100}%`;
});

/** Which row of the rail a viewport Y coordinate falls on. */
function indexFromY(clientY: number): number {
  const el = rail.value;
  if (!el) return -1;
  const rect = el.getBoundingClientRect();
  if (rect.height <= 0) return -1;
  const ratio = (clientY - rect.top) / rect.height;
  return Math.max(0, Math.min(props.entries.length - 1, Math.floor(ratio * props.entries.length)));
}

function preview(label: string) {
  active.value = label;
  window.clearTimeout(hideTimer);
  hideTimer = window.setTimeout(() => { if (!dragging.value) active.value = null; }, 800);
}

/** Jump to the bucket under a Y coordinate, if it holds any tracks. */
function pickAt(clientY: number) {
  const index = indexFromY(clientY);
  if (index < 0) return;
  const entry = props.entries[index];
  const now = Date.now();
  if (lastPick && lastPick.label === entry.label && now - lastPick.at < 300) return;
  lastPick = { label: entry.label, at: now };
  preview(entry.label);
  if (entry.present) emit("select", entry);
}

function onPointerDown(event: PointerEvent) {
  dragging.value = true;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  pickAt(event.clientY);
}

function onPointerMove(event: PointerEvent) {
  if (dragging.value) pickAt(event.clientY);
}

function onPointerUp(event: PointerEvent) {
  dragging.value = false;
  const el = event.currentTarget as HTMLElement;
  if (el.hasPointerCapture?.(event.pointerId)) el.releasePointerCapture(event.pointerId);
  hideTimer = window.setTimeout(() => { active.value = null; }, 600);
}
</script>

<template>
  <div
    ref="rail"
    class="track-index"
    :class="dragging ? 'track-index--dragging' : ''"
    role="listbox"
    :aria-label="t('library.titleIndex')"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @pointerleave="onPointerUp"
  >
    <span class="track-index__bubble" :style="{ top: activeTop }" :class="active ? 'track-index__bubble--visible' : ''">{{ active }}</span>
    <button
      v-for="entry in entries"
      :key="entry.label"
      type="button"
      class="track-index__item"
      :class="[entry.present ? '' : 'track-index__item--empty', active === entry.label ? 'track-index__item--active' : '']"
      :disabled="!entry.present"
      :aria-selected="active === entry.label"
      @mouseenter="entry.present && preview(entry.label)"
    >{{ entry.label }}</button>
  </div>
</template>

<style scoped>
.track-index {
  position: fixed;
  top: 50%;
  right: 0.4rem;
  z-index: 15;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.03rem;
  padding: 0.25rem 0.1rem;
  transform: translateY(-50%);
  touch-action: none;
  user-select: none;
}

.track-index__item {
  display: grid;
  place-items: center;
  width: 1.05rem;
  height: 1.05rem;
  font-family: var(--font-mono);
  font-size: 0.62rem;
  font-weight: 700;
  line-height: 1;
  color: var(--dim);
  transition: color 160ms cubic-bezier(0.2, 0.8, 0.2, 1), transform 160ms cubic-bezier(0.2, 0.8, 0.2, 1);
}

.track-index__item:hover:not(:disabled),
.track-index__item--active {
  color: var(--accent);
  transform: scale(1.35);
}

.track-index__item--empty {
  color: color-mix(in srgb, var(--dim) 40%, transparent);
}

/* The bubble mirrors the active letter while scrubbing or hovering. */
.track-index__bubble {
  position: absolute;
  right: calc(100% + 0.4rem);
  display: grid;
  place-items: center;
  width: 2.6rem;
  height: 2.6rem;
  margin-top: -1.3rem;
  background: var(--accent);
  color: var(--accent-fg);
  font-family: var(--font-mono);
  font-size: 1.35rem;
  font-weight: 800;
  opacity: 0;
  transform: scale(0.7);
  pointer-events: none;
  transition: opacity 160ms ease, transform 160ms cubic-bezier(0.2, 0.8, 0.2, 1);
  clip-path: polygon(0 0, calc(100% - 8px) 0, 100% 8px, 100% 100%, 8px 100%, 0 calc(100% - 8px));
}

.track-index__bubble--visible {
  opacity: 1;
  transform: scale(1);
}

@media (max-width: 767.98px) {
  /* Phones are too short for 27 letters; the rail condenses to fit. */
  .track-index__item {
    width: 0.95rem;
    height: 0.72rem;
    font-size: 0.55rem;
  }
}

@media (max-height: 640px) {
  /* Short windows: tighten the rail so all 27 rows still fit. */
  .track-index {
    gap: 0;
    padding: 0.1rem 0.1rem;
  }

  .track-index__item {
    height: 0.85rem;
    font-size: 0.55rem;
  }
}
</style>
