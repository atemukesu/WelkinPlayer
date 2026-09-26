<script setup lang="ts">
import { LyricPlayer } from "@applemusic-like-lyrics/vue";
import "@applemusic-like-lyrics/core/style.css";
import type { LyricLine, LyricLineMouseEvent } from "@applemusic-like-lyrics/core";

const props = withDefaults(
  defineProps<{
    lines: LyricLine[];
    currentTime: number;
    playing: boolean;
    fontSize: number;
    color?: string;
    blend?: "normal" | "plus-lighter";
  }>(),
  { color: "var(--fg)", blend: "normal" },
);

const emit = defineEmits<{ seek: [timeMs: number] }>();

/** Clicking a line seeks to that line's start time. */
function onLineClick(event: LyricLineMouseEvent) {
  const timeMs = event.line?.getLine().startTime;
  if (typeof timeMs === "number" && Number.isFinite(timeMs)) emit("seek", timeMs);
}
</script>

<template>
  <LyricPlayer
    class="amll-lyrics"
    :style="{ '--amll-lp-color': props.color, mixBlendMode: props.blend, '--amll-lp-font-size': `${props.fontSize}px` }"
    :lyric-lines="props.lines"
    :current-time="props.currentTime"
    :playing="props.playing"
    align-anchor="center"
    :word-fade-width="0.5"
    @line-click="onLineClick"
  />
</template>

<style scoped>
.amll-lyrics {
  width: 100%;
  height: 100%;
}
</style>
