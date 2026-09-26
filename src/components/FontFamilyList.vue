<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDown, ChevronUp, X } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { cssFontFamily } from "../lib/fonts";
import LayeredSelect from "./LayeredSelect.vue";

const props = defineProps<{ modelValue: string[]; available: string[] }>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();
const { t } = useI18n();

const pending = ref("");

/** Fonts not already in the ordered list, plus a placeholder that keeps the trigger labelled. */
const options = computed(() => [
  { value: "", label: t("settings.lyrics.fontSelect") },
  ...props.available
    .filter((font) => !props.modelValue.includes(font))
    .map((font) => ({ value: font, label: font })),
]);

/** Selecting a font adds it to the end of the stack and resets the trigger. */
function select(value: string) {
  if (value && !props.modelValue.includes(value)) {
    emit("update:modelValue", [...props.modelValue, value]);
  }
  pending.value = "";
}

function remove(index: number) {
  emit("update:modelValue", props.modelValue.filter((_, itemIndex) => itemIndex !== index));
}

function move(index: number, direction: -1 | 1) {
  const target = index + direction;
  if (target < 0 || target >= props.modelValue.length) return;
  const next = [...props.modelValue];
  [next[index], next[target]] = [next[target], next[index]];
  emit("update:modelValue", next);
}
</script>

<template>
  <div class="grid gap-2">
    <LayeredSelect
      :model-value="pending"
      :label="t('settings.lyrics.fontSelect')"
      :options="options"
      @update:model-value="select"
    />

    <ul v-if="modelValue.length" class="grid min-w-0 gap-1">
      <li
        v-for="(font, index) in modelValue"
        :key="`${font}-${index}`"
        class="flex min-w-0 items-center gap-2 border border-line bg-bg px-2 py-1.5"
      >
        <span class="w-6 shrink-0 text-center font-mono text-[11px] text-dim">{{ index + 1 }}</span>
        <span class="min-w-0 flex-1 truncate text-[13px] font-normal normal-case tracking-normal text-fg" :style="{ fontFamily: cssFontFamily([font]) }">{{ font }}</span>
        <button
          type="button"
          class="grid h-7 w-7 place-items-center text-dim transition hover:text-fg disabled:opacity-30"
          :disabled="index === 0"
          :title="t('settings.lyrics.fontMoveUp')"
          @click="move(index, -1)"
        >
          <ChevronUp :size="14" />
        </button>
        <button
          type="button"
          class="grid h-7 w-7 place-items-center text-dim transition hover:text-fg disabled:opacity-30"
          :disabled="index === modelValue.length - 1"
          :title="t('settings.lyrics.fontMoveDown')"
          @click="move(index, 1)"
        >
          <ChevronDown :size="14" />
        </button>
        <button
          type="button"
          class="grid h-7 w-7 place-items-center text-dim transition hover:text-accent"
          :title="t('settings.lyrics.fontRemove')"
          @click="remove(index)"
        >
          <X :size="14" />
        </button>
      </li>
    </ul>
    <p v-else class="text-[11px] font-normal normal-case tracking-normal text-dim">{{ t("settings.lyrics.fontEmpty") }}</p>
  </div>
</template>

<style scoped>
/* Font names are proper nouns — keep them readable inside the shared dropdown. */
:deep(.layered-select__option) {
  letter-spacing: normal;
  text-transform: none;
}
</style>
