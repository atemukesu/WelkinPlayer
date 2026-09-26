<script setup lang="ts">
import { computed } from "vue";
import { LyricPlayer } from "@applemusic-like-lyrics/vue";
import "@applemusic-like-lyrics/core/style.css";
import type { LyricLine, LyricLineMouseEvent } from "@applemusic-like-lyrics/core";
import { cssFontFamily } from "../lib/fonts";

const props = withDefaults(
  defineProps<{
    lines: LyricLine[];
    currentTime: number;
    playing: boolean;
    fontSize: number;
    /** CSS font weight applied to the lyric text. */
    fontWeight?: number;
    /** Ordered font-family fallback list; empty uses the app font. */
    fontFamilies?: string[];
    /** Sub-line (translation) size in px; `undefined` keeps AMLL's default. */
    translationSize?: number;
    color?: string;
    blend?: "normal" | "plus-lighter";
    /** Vertical position (0–1 from the top) the active line is aligned to. */
    alignPosition?: number;
  }>(),
  { color: "var(--fg)", blend: "normal", alignPosition: 0.5, fontWeight: 400, fontFamilies: () => [] },
);

const emit = defineEmits<{ seek: [timeMs: number] }>();

const fontFamily = computed(() => cssFontFamily(props.fontFamilies));

const playerStyle = computed(() => ({
  "--amll-lp-color": props.color,
  "--amll-lp-font-size": `${props.fontSize}px`,
  "--amll-translation-size": props.translationSize ? `${props.translationSize}px` : undefined,
  mixBlendMode: props.blend,
  fontFamily: fontFamily.value,
  fontWeight: String(props.fontWeight),
}));

/** Clicking a line seeks to that line's start time. */
function onLineClick(event: LyricLineMouseEvent) {
  const timeMs = event.line?.getLine().startTime;
  if (typeof timeMs === "number" && Number.isFinite(timeMs)) emit("seek", timeMs);
}
</script>

<template>
  <LyricPlayer
    class="amll-lyrics"
    :style="playerStyle"
    :lyric-lines="props.lines"
    :current-time="props.currentTime"
    :playing="props.playing"
    align-anchor="center"
    :align-position="props.alignPosition"
    :word-fade-width="0.5"
    @line-click="onLineClick"
  />
</template>

<style scoped>
.amll-lyrics {
  width: 100%;
  height: 100%;
}

/* AMLL hard-codes the translation sub-line at .5em; let the user override it. */
.amll-lyrics :deep(.FmKaba_lyricSubLine) {
  font-size: var(--amll-translation-size, max(0.5em, 10px));
}
</style>
