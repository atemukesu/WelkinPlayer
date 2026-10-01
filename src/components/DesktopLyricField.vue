<script setup lang="ts" generic="T extends string | number | boolean">
import { computed } from "vue";

/**
 * One row of the desktop-lyrics settings: a labelled slider, colour swatch,
 * toggle or select. Kept generic so the settings panel stays declarative.
 */
const props = withDefaults(
  defineProps<{
    label: string;
    type: "slider" | "color" | "toggle" | "select";
    hint?: string;
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
    options?: Array<{ value: string; label: string }>;
    disabled?: boolean;
  }>(),
  { type: "slider", min: 0, max: 100, step: 1, unit: "", disabled: false },
);

const model = defineModel<T>({ required: true });

const fill = computed(() => {
  if (props.type !== "slider") return "0%";
  const value = Number(model.value);
  const span = props.max - props.min || 1;
  return `${Math.min(100, Math.max(0, ((value - props.min) / span) * 100))}%`;
});
</script>

<template>
  <label v-if="props.type === 'toggle'" class="flex items-center justify-between gap-4">
    <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
      {{ props.label }}
      <span v-if="props.hint" class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ props.hint }}</span>
    </span>
    <input v-model="model" class="ak-switch shrink-0" type="checkbox" :disabled="props.disabled" />
  </label>

  <label v-else-if="props.type === 'color'" class="flex items-center justify-between gap-4">
    <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
      {{ props.label }}
      <span v-if="props.hint" class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ props.hint }}</span>
    </span>
    <span class="flex items-center gap-2">
      <output class="font-mono text-[11px] uppercase text-dim">{{ model }}</output>
      <input
        v-model="model"
        type="color"
        class="h-7 w-9 cursor-pointer border border-line bg-transparent p-0"
        :disabled="props.disabled"
      />
    </span>
  </label>

  <label v-else-if="props.type === 'select'" class="flex items-center justify-between gap-4">
    <span class="grid gap-1 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
      {{ props.label }}
      <span v-if="props.hint" class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ props.hint }}</span>
    </span>
    <select
      v-model="model"
      class="h-9 border border-line bg-bg px-3 text-sm font-normal normal-case tracking-normal text-fg outline-none focus:border-accent"
      :disabled="props.disabled"
    >
      <option v-for="option in props.options" :key="option.value" :value="option.value">{{ option.label }}</option>
    </select>
  </label>

  <label v-else class="grid gap-2 text-[13px] font-semibold uppercase tracking-[0.2em] text-dim">
    <span class="flex items-center justify-between">
      <span>
        {{ props.label }}
        <span v-if="props.hint" class="ml-2 text-[11px] font-normal normal-case tracking-normal text-dim">{{ props.hint }}</span>
      </span>
      <output class="font-mono text-accent">{{ model }}{{ props.unit }}</output>
    </span>
    <input
      v-model.number="model"
      class="ak-slider"
      type="range"
      :min="props.min"
      :max="props.max"
      :step="props.step"
      :style="{ '--fill': fill }"
      :disabled="props.disabled"
    />
  </label>
</template>
