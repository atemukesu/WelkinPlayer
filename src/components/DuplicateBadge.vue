<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { Copy } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import type { Track } from "../stores/player";
import { trackOrigin } from "../lib/remote";

const props = defineProps<{ track: Track }>();
const { t } = useI18n();

const anchor = ref<HTMLButtonElement | null>(null);
const hovered = ref(false);
const pinned = ref(false);
const open = computed(() => hovered.value || pinned.value);

const top = ref(0);
const left = ref(0);
const above = ref(true);

const hasCopies = computed(() => (props.track.duplicates?.length ?? 0) > 0);
/** Own location first, then the other files with the same name and byte size. */
const hint = computed(() => {
  const copies = props.track.duplicates ?? [];
  if (copies.length === 0) return "";
  const origin = trackOrigin(props.track);
  const lines: string[] = [];
  if (origin) lines.push(t("library.origin", { origin }));
  lines.push(t("library.duplicateAt", { origins: copies.join(" · ") }));
  return lines.join("\n");
});

/** Anchor the tooltip above the icon, or below it when there is no room. */
function place() {
  const el = anchor.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  above.value = rect.top >= 72;
  top.value = above.value ? rect.top - 6 : rect.bottom + 6;
  left.value = Math.min(Math.max(rect.left + rect.width / 2, 12), window.innerWidth - 12);
}

function show() {
  place();
  hovered.value = true;
}

function hide() {
  hovered.value = false;
  pinned.value = false;
}

/** Tap/click pins the tooltip so it also works without hover (touch screens). */
function toggle(event: MouseEvent) {
  event.stopPropagation();
  event.preventDefault();
  pinned.value = !pinned.value;
  if (pinned.value) place();
}

function onPointerDown(event: PointerEvent) {
  if (!pinned.value) return;
  const el = anchor.value;
  if (el && event.target instanceof Node && el.contains(event.target)) return;
  pinned.value = false;
}

function onScroll() {
  pinned.value = false;
}

onMounted(() => {
  document.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("scroll", onScroll, true);
});
onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("scroll", onScroll, true);
});
</script>

<template>
  <button
    v-if="hasCopies"
    ref="anchor"
    type="button"
    class="grid shrink-0 place-items-center text-amber-400 transition-colors hover:text-amber-300"
    :aria-label="hint"
    :aria-expanded="open"
    @mouseenter="show"
    @mouseleave="hovered = false"
    @click="toggle"
    @keydown.escape="hide"
  >
    <Copy :size="13" :stroke-width="2" />
  </button>
  <Teleport to="body">
    <span
      v-if="hasCopies && open"
      role="tooltip"
      class="pointer-events-none fixed z-[70] max-w-[min(80vw,320px)] whitespace-pre-line border border-line bg-bg px-2 py-1 text-[11px] leading-snug text-fg shadow-lg"
      :style="{ top: `${top}px`, left: `${left}px`, transform: `translate(-50%, ${above ? '-100%' : '0'})` }"
    >{{ hint }}</span>
  </Teleport>
</template>
