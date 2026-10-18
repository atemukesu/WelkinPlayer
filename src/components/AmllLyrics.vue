<!--
 Copyright 2026 Atemukesu
 SPDX-License-Identifier: GPL-3.0-only
-->

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
    /** Audio playback rate; AMLL's own animations always run at 1× (see below). */
    rate?: number;
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
  { color: "var(--fg)", blend: "normal", alignPosition: 0.5, fontWeight: 400, fontFamilies: () => [], rate: 1 },
);

const emit = defineEmits<{ seek: [timeMs: number] }>();

const fontFamily = computed(() => cssFontFamily(props.fontFamilies));

/** Playback rate, sanitized: anything non-finite or non-positive means 1×. */
const rate = computed(() => {
  const value = Number(props.rate);
  return Number.isFinite(value) && value > 0 ? value : 1;
});

/**
 * AMLL advances its per-word highlight with Web Animations / CSS masks that
 * always run at 1× wall-clock, and it only re-anchors them when a line starts
 * singing. It therefore expects one media second per real second: at 2× the
 * karaoke sweep would only reach the middle of the line when the line ends,
 * and the same lag appears (in the other direction) when slowing down.
 *
 * Rather than reaching into AMLL's private animators, hand it a timeline that
 * is already divided by the playback rate (`t' = t / rate`) and push
 * `currentTime / rate` as the playhead. On that timeline a second of playback
 * really does last a second of wall time, so AMLL's 1× animations stay glued
 * to the audio at any speed while line changes still fire at the right moment.
 *
 * The trade-off is that anything AMLL derives from timeline *spans* (the
 * interlude-dot choreography, the per-gap spring tuning) is based on the
 * shortened timeline too, so at non-1× speeds the interlude dots finish their
 * routine early. The karaoke sweep itself — the part that was visibly wrong —
 * is now exact.
 */
const scaledLines = computed<LyricLine[]>(() => {
  const factor = 1 / rate.value;
  if (factor === 1) return props.lines;
  return props.lines.map((line) => ({
    ...line,
    startTime: line.startTime * factor,
    endTime: line.endTime * factor,
    words: line.words.map((word) => {
      const next = { ...word, startTime: word.startTime * factor, endTime: word.endTime * factor };
      if (word.ruby?.length) {
        next.ruby = word.ruby.map((ruby) => ({ ...ruby, startTime: ruby.startTime * factor, endTime: ruby.endTime * factor }));
      }
      return next;
    }),
  }));
});

/** Playhead on the scaled timeline AMLL is rendering. */
const scaledTime = computed(() => props.currentTime / rate.value);

const playerStyle = computed(() => ({
  "--amll-lp-color": props.color,
  "--amll-lp-font-size": `${props.fontSize}px`,
  "--amll-translation-size": props.translationSize ? `${props.translationSize}px` : undefined,
  mixBlendMode: props.blend,
  fontFamily: fontFamily.value,
  fontWeight: String(props.fontWeight),
}));

/** Clicking a line seeks to that line's start time (mapped back to media time). */
function onLineClick(event: LyricLineMouseEvent) {
  const timeMs = event.line?.getLine().startTime;
  if (typeof timeMs === "number" && Number.isFinite(timeMs)) emit("seek", timeMs * rate.value);
}
</script>

<template>
  <LyricPlayer
    class="amll-lyrics"
    :style="playerStyle"
    :lyric-lines="scaledLines"
    :current-time="scaledTime"
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
