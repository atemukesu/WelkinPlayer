<script setup lang="ts">
import { ref } from "vue";
import { ChevronDown, ChevronUp, GripVertical } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import type { LyricProvider } from "../lib/preferences";

const props = defineProps<{ modelValue: LyricProvider[]; disabled?: boolean }>();
const emit = defineEmits<{ "update:modelValue": [value: LyricProvider[]] }>();
const { t } = useI18n();

/** Index currently being dragged, or null when idle. */
const dragIndex = ref<number | null>(null);
/** Index the dragged row is hovering over (the drop target). */
const overIndex = ref<number | null>(null);

/** Human-readable name shown on each row. */
function label(provider: LyricProvider): string {
  return t(`settings.lyrics.provider.${provider}.name`);
}

/** Supporting description shown under each row. */
function description(provider: LyricProvider): string {
  return t(`settings.lyrics.provider.${provider}.desc`);
}

function onDragStart(index: number, event: DragEvent) {
  dragIndex.value = index;
  overIndex.value = index;
  if (event.dataTransfer) {
    event.dataTransfer.effectAllowed = "move";
    // Firefox needs data set for the drag to start.
    event.dataTransfer.setData("text/plain", String(index));
  }
}

function onDragOver(index: number, event: DragEvent) {
  if (dragIndex.value === null) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  overIndex.value = index;
}

function onDrop(index: number) {
  const from = dragIndex.value;
  reset();
  if (from === null || from === index) return;
  const next = [...props.modelValue];
  const [moved] = next.splice(from, 1);
  next.splice(index, 0, moved);
  emit("update:modelValue", next);
}

function reset() {
  dragIndex.value = null;
  overIndex.value = null;
}

/** Keyboard/accessible fallback: swap the row one slot up or down. */
function move(index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= props.modelValue.length) return;
  const next = [...props.modelValue];
  [next[index], next[target]] = [next[target], next[index]];
  emit("update:modelValue", next);
}
</script>

<template>
  <ul class="grid gap-1.5" @dragend="reset">
    <li
      v-for="(provider, index) in modelValue"
      :key="provider"
      class="flex items-stretch gap-3 border bg-bg px-3 py-2.5 transition-colors"
      :class="[
        dragIndex === index ? 'border-accent opacity-50' : 'border-line',
        overIndex === index && dragIndex !== null && dragIndex !== index ? 'border-accent bg-accent/5' : '',
        disabled ? 'pointer-events-none opacity-40' : 'cursor-grab active:cursor-grabbing',
      ]"
      :draggable="!disabled"
      @dragstart="onDragStart(index, $event)"
      @dragover="onDragOver(index, $event)"
      @drop.prevent="onDrop(index)"
    >
      <span class="flex w-5 shrink-0 items-center justify-center text-dim" :title="t('settings.lyrics.providerDrag')">
        <GripVertical :size="16" :stroke-width="1.8" />
      </span>
      <span class="w-6 shrink-0 self-center text-center font-mono text-[11px] text-dim">{{ index + 1 }}</span>
      <span class="grid min-w-0 flex-1 gap-1">
        <span class="text-[13px] font-semibold uppercase tracking-[0.2em] text-fg">{{ label(provider) }}</span>
        <span class="text-[11px] font-normal normal-case leading-relaxed tracking-normal text-dim">{{ description(provider) }}</span>
      </span>
      <span class="flex shrink-0 items-center gap-1">
        <button
          type="button"
          class="grid h-7 w-7 place-items-center text-dim transition hover:text-fg disabled:opacity-30"
          :disabled="disabled || index === 0"
          :title="t('settings.lyrics.providerMoveUp')"
          @click="move(index, -1)"
        >
          <ChevronUp :size="14" />
        </button>
        <button
          type="button"
          class="grid h-7 w-7 place-items-center text-dim transition hover:text-fg disabled:opacity-30"
          :disabled="disabled || index === modelValue.length - 1"
          :title="t('settings.lyrics.providerMoveDown')"
          @click="move(index, 1)"
        >
          <ChevronDown :size="14" />
        </button>
      </span>
    </li>
  </ul>
</template>
