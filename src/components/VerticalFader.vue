<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, ref } from "vue";

/**
 * Vertical fader drawn as a thin progress rail.
 *
 * The fill grows from the centre (0 dB) towards the thumb, which is the shape a
 * graphic EQ reads best. Pointer drags update the model live; `commit` fires on
 * release so callers can apply the value once instead of retuning the engine on
 * every step. Keyboard arrows/Home/End work on the focused rail.
 */
const props = withDefaults(
  defineProps<{
    modelValue: number;
    min: number;
    max: number;
    step?: number;
    disabled?: boolean;
    label?: string;
  }>(),
  { step: 1, disabled: false, label: "" },
);

const emit = defineEmits<{
  "update:modelValue": [value: number];
  commit: [];
}>();

const track = ref<HTMLElement | null>(null);
let pointerId: number | null = null;
let changed = false;

const range = computed(() => props.max - props.min);

function onCurve(value: number): number {
  return range.value > 0
    ? Math.min(1, Math.max(0, (value - props.min) / range.value))
    : 0;
}

/** Current value as a share of the rail, bottom (0) to top (1). */
const ratio = computed(() => onCurve(props.modelValue));
/** Where 0 dB would sit on the rail. */
const zeroRatio = computed(() => onCurve(0));

const fillStyle = computed(() => {
  const bottom = Math.min(ratio.value, zeroRatio.value);
  const top = Math.max(ratio.value, zeroRatio.value);
  return { bottom: `${bottom * 100}%`, height: `${(top - bottom) * 100}%` };
});

function clamp(value: number): number {
  if (!Number.isFinite(value)) return props.modelValue;
  const steps = Math.round((value - props.min) / props.step);
  const stepped = props.min + steps * props.step;
  return Number(
    Math.min(props.max, Math.max(props.min, stepped)).toFixed(6),
  );
}

function valueAt(clientY: number): number {
  const element = track.value;
  if (!element) return props.modelValue;
  const rect = element.getBoundingClientRect();
  const share = rect.height > 0 ? 1 - (clientY - rect.top) / rect.height : 0;
  return clamp(props.min + Math.min(1, Math.max(0, share)) * range.value);
}

function apply(clientY: number) {
  const next = valueAt(clientY);
  if (next !== props.modelValue) {
    emit("update:modelValue", next);
    changed = true;
  }
}

function onPointerDown(event: PointerEvent) {
  if (props.disabled || event.button !== 0) return;
  event.preventDefault();
  pointerId = event.pointerId;
  changed = false;
  const element = event.currentTarget as HTMLElement;
  element.setPointerCapture?.(event.pointerId);
  // `preventScroll` keeps the modal from jumping when a rail near the edge is
  // focused; a scroll during a drag makes the fader and scrollbar twitch.
  element.focus({ preventScroll: true });
  apply(event.clientY);
}

function onPointerMove(event: PointerEvent) {
  if (pointerId !== event.pointerId) return;
  apply(event.clientY);
}

function onPointerUp(event: PointerEvent) {
  if (pointerId !== event.pointerId) return;
  pointerId = null;
  if (changed) emit("commit");
  changed = false;
}

function onKeydown(event: KeyboardEvent) {
  if (props.disabled) return;
  let next: number | null = null;
  switch (event.key) {
    case "ArrowUp":
    case "ArrowRight":
      next = clamp(props.modelValue + props.step);
      break;
    case "ArrowDown":
    case "ArrowLeft":
      next = clamp(props.modelValue - props.step);
      break;
    case "PageUp":
      next = clamp(props.modelValue + props.step * 4);
      break;
    case "PageDown":
      next = clamp(props.modelValue - props.step * 4);
      break;
    case "Home":
      next = props.min;
      break;
    case "End":
      next = props.max;
      break;
  }
  if (next === null || next === props.modelValue) return;
  event.preventDefault();
  emit("update:modelValue", next);
  emit("commit");
}
</script>

<template>
  <div
    ref="track"
    class="vf"
    :class="{ 'vf--disabled': disabled }"
    role="slider"
    :tabindex="disabled ? -1 : 0"
    aria-orientation="vertical"
    :aria-valuemin="min"
    :aria-valuemax="max"
    :aria-valuenow="modelValue"
    :aria-label="label || undefined"
    :aria-disabled="disabled ? 'true' : 'false'"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKeydown"
  >
    <span class="vf__rail" aria-hidden="true" />
    <span class="vf__zero" aria-hidden="true" />
    <span class="vf__fill" :style="fillStyle" aria-hidden="true" />
    <span class="vf__handle" :style="{ bottom: `${ratio * 100}%` }" aria-hidden="true" />
  </div>
</template>

<style scoped>
.vf {
  position: relative;
  width: 28px;
  height: 148px;
  outline: none;
  cursor: ns-resize;
  touch-action: none;
  -webkit-user-select: none;
  user-select: none;
}

.vf__rail {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 3px;
  transform: translateX(-50%);
  background: var(--line);
}

.vf__zero {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 14px;
  height: 1px;
  transform: translate(-50%, -50%);
  background: var(--line-strong);
  opacity: 0.55;
}

.vf__fill {
  position: absolute;
  left: 50%;
  width: 3px;
  transform: translateX(-50%);
  background: var(--accent);
}

.vf__handle {
  position: absolute;
  left: 50%;
  width: 15px;
  height: 3px;
  transform: translate(-50%, 50%);
  background: var(--accent);
  transition: width 140ms ease, height 140ms ease;
}

.vf:hover .vf__handle,
.vf:focus-visible .vf__handle {
  width: 19px;
  height: 4px;
}

.vf:focus-visible {
  outline: 1px solid var(--accent);
  outline-offset: 4px;
}

.vf--disabled {
  opacity: 0.4;
  cursor: default;
}
</style>
