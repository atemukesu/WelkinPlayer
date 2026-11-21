<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref } from "vue";

/**
 * Rotary knob with a 270° sweep.
 *
 * Pointer drags are relative (vertical) instead of absolute, so small knobs can
 * still reach every value without the pointer leaving the dial. As with the
 * faders, `commit` fires on release or on a batch of key presses, letting the
 * caller apply the value once. `origin` draws a tick at the factory position;
 * double-clicking the dial returns to it.
 */
const props = withDefaults(
  defineProps<{
    modelValue: number;
    min: number;
    max: number;
    step?: number;
    /** Factory value, marked with a tick on the ring when it is a number. */
    origin?: number;
    unit?: string;
    label?: string;
    hint?: string;
    disabled?: boolean;
    /** Optional value formatter, e.g. one decimal for a dB threshold. */
    format?: (value: number) => string;
  }>(),
  { step: 1, origin: Number.NaN, unit: "", label: "", hint: "", disabled: false, format: undefined },
);

const emit = defineEmits<{
  "update:modelValue": [value: number];
  commit: [];
}>();

const START_ANGLE = 135;
const SWEEP = 270;
const CENTER = 30;
const ARC_RADIUS = 25;
/** Pixels of vertical travel for a full sweep. */
const DRAG_SPAN = 140;

const drag = ref<{ pointer: number; startY: number; startValue: number } | null>(null);
let changed = false;

const range = computed(() => props.max - props.min);

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

const ratio = computed(() =>
  range.value > 0 ? clamp01((props.modelValue - props.min) / range.value) : 0,
);

const originRatio = computed(() =>
  range.value > 0 && Number.isFinite(props.origin)
    ? clamp01((props.origin - props.min) / range.value)
    : null,
);

const display = computed(() =>
  props.format ? props.format(props.modelValue) : `${Math.round(props.modelValue)}${props.unit}`,
);

function polar(angle: number, radius: number): [number, number] {
  const radians = (angle * Math.PI) / 180;
  return [CENTER + radius * Math.cos(radians), CENTER + radius * Math.sin(radians)];
}

/** SVG arc from `from` to `to` degrees along the dial ring. */
function arc(from: number, to: number): string {
  const [sx, sy] = polar(from, ARC_RADIUS);
  const [ex, ey] = polar(to, ARC_RADIUS);
  const large = to - from > 180 ? 1 : 0;
  return `M ${sx} ${sy} A ${ARC_RADIUS} ${ARC_RADIUS} 0 ${large} 1 ${ex} ${ey}`;
}

const trackPath = arc(START_ANGLE, START_ANGLE + SWEEP);
const valuePath = computed(() => {
  const to = START_ANGLE + ratio.value * SWEEP;
  return to - START_ANGLE < 0.5 ? "" : arc(START_ANGLE, to);
});

const pointer = computed(() => {
  const angle = START_ANGLE + ratio.value * SWEEP;
  const [x1, y1] = polar(angle, 5);
  const [x2, y2] = polar(angle, 13.5);
  return { x1, y1, x2, y2 };
});

const originTick = computed(() => {
  if (originRatio.value === null) return null;
  const angle = START_ANGLE + originRatio.value * SWEEP;
  const [x1, y1] = polar(angle, 27.5);
  const [x2, y2] = polar(angle, 31);
  return { x1, y1, x2, y2 };
});

function snap(value: number): number {
  if (!Number.isFinite(value)) return props.modelValue;
  const steps = Math.round((value - props.min) / props.step);
  const stepped = props.min + steps * props.step;
  return Number(Math.min(props.max, Math.max(props.min, stepped)).toFixed(6));
}

function set(next: number) {
  if (next === props.modelValue) return false;
  emit("update:modelValue", next);
  changed = true;
  return true;
}

function onPointerDown(event: PointerEvent) {
  if (props.disabled || event.button !== 0) return;
  event.preventDefault();
  const element = event.currentTarget as HTMLElement;
  element.setPointerCapture?.(event.pointerId);
  element.focus({ preventScroll: true });
  drag.value = {
    pointer: event.pointerId,
    startY: event.clientY,
    startValue: props.modelValue,
  };
  changed = false;
}

function onPointerMove(event: PointerEvent) {
  const state = drag.value;
  if (!state || state.pointer !== event.pointerId) return;
  const scale = event.shiftKey ? 0.25 : 1;
  const delta = ((state.startY - event.clientY) / DRAG_SPAN) * range.value * scale;
  set(snap(state.startValue + delta));
}

function onPointerUp(event: PointerEvent) {
  const state = drag.value;
  if (!state || state.pointer !== event.pointerId) return;
  drag.value = null;
  if (changed) emit("commit");
  changed = false;
}

function onDoubleClick() {
  if (props.disabled || !Number.isFinite(props.origin)) return;
  const next = snap(props.origin);
  if (next === props.modelValue) return;
  changed = false;
  emit("update:modelValue", next);
  emit("commit");
}

function onKeydown(event: KeyboardEvent) {
  if (props.disabled) return;
  let next: number | null = null;
  switch (event.key) {
    case "ArrowUp":
    case "ArrowRight":
      next = snap(props.modelValue + props.step);
      break;
    case "ArrowDown":
    case "ArrowLeft":
      next = snap(props.modelValue - props.step);
      break;
    case "PageUp":
      next = snap(props.modelValue + props.step * 10);
      break;
    case "PageDown":
      next = snap(props.modelValue - props.step * 10);
      break;
    case "Home":
      next = props.min;
      break;
    case "End":
      next = props.max;
      break;
  }
  if (next === null) return;
  event.preventDefault();
  if (set(next)) emit("commit");
}
</script>

<template>
  <div class="knob" :class="{ 'knob--disabled': disabled }" :title="hint || undefined">
    <div
      class="knob__dial"
      role="slider"
      :tabindex="disabled ? -1 : 0"
      :aria-valuemin="min"
      :aria-valuemax="max"
      :aria-valuenow="modelValue"
      :aria-valuetext="display"
      :aria-label="label || undefined"
      :aria-disabled="disabled ? 'true' : 'false'"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @dblclick="onDoubleClick"
      @keydown="onKeydown"
    >
      <svg viewBox="0 0 60 60" aria-hidden="true">
        <path class="knob__track" :d="trackPath" />
        <path v-if="valuePath" class="knob__value" :d="valuePath" />
        <line
          v-if="originTick"
          class="knob__origin"
          :x1="originTick.x1"
          :y1="originTick.y1"
          :x2="originTick.x2"
          :y2="originTick.y2"
        />
        <circle class="knob__body" :cx="CENTER" :cy="CENTER" r="15.5" />
        <line
          class="knob__pointer"
          :x1="pointer.x1"
          :y1="pointer.y1"
          :x2="pointer.x2"
          :y2="pointer.y2"
        />
      </svg>
    </div>
    <span class="knob__text">
      <span class="knob__name">{{ label }}</span>
      <span class="knob__reading">{{ display }}</span>
    </span>
  </div>
</template>

<style scoped>
.knob {
  display: flex;
  width: 56px;
  flex-direction: column;
  align-items: center;
  gap: 4px;
}

.knob__dial {
  width: 52px;
  height: 52px;
  border-radius: 9999px;
  outline: none;
  cursor: ns-resize;
  touch-action: none;
  -webkit-user-select: none;
  user-select: none;
}

.knob__dial svg {
  display: block;
  width: 100%;
  height: 100%;
  overflow: visible;
}

.knob__track {
  fill: none;
  stroke: var(--line-strong);
  stroke-width: 2.5;
  stroke-linecap: round;
  opacity: 0.45;
}

.knob__value {
  fill: none;
  stroke: var(--accent);
  stroke-width: 2.5;
  stroke-linecap: round;
}

.knob__origin {
  stroke: var(--dim);
  stroke-width: 1.5;
  opacity: 0.95;
}

.knob__body {
  fill: var(--surface);
  stroke: var(--line-strong);
  stroke-width: 1;
  transition: stroke 140ms ease;
}

.knob__pointer {
  stroke: var(--fg);
  stroke-width: 2;
  stroke-linecap: round;
}

.knob:hover .knob__body,
.knob__dial:focus-visible .knob__body {
  stroke: var(--accent);
}

.knob__dial:focus-visible {
  outline: 1px solid var(--accent);
  outline-offset: 2px;
}

.knob__text {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  line-height: 1.15;
}

.knob__name {
  font-size: 9px;
  font-weight: 600;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--dim);
}

.knob__reading {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  color: var(--accent);
}

.knob--disabled {
  opacity: 0.4;
}

.knob--disabled .knob__dial {
  cursor: default;
}
</style>
