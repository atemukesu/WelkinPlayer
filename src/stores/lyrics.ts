import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { findLyricIndex, parseLyric } from "lyric-kit";
import { invoke, toAppError } from "../api";

export type LyricsSource = "local" | "disabled";
export type LyricsStatus = "idle" | "loading" | "ready" | "error";

export type LyricLine = ReturnType<typeof parseLyric>["lines"][number];

export const useLyricsStore = defineStore("lyrics", () => {
  const source = ref<LyricsSource>("local");
  const translate = ref(true);
  function readSetting(key: string, min: number, max: number, fallback: number) {
    const value = Number(localStorage.getItem(key));
    return Number.isFinite(value) && value >= min && value <= max ? value : fallback;
  }
  const lineSize = ref(readSetting("welkin-lyrics-size", 16, 48, 24));
  const translationSize = ref(readSetting("welkin-lyrics-translation-size", 12, 32, 18));
  const lineSpacing = ref(readSetting("welkin-lyrics-spacing", 8, 36, 24));
  const status = ref<LyricsStatus>("idle");
  const error = ref("");
  const lines = ref<LyricLine[]>([]);
  const activeIndex = ref(-1);

  const enabled = computed(() => source.value !== "disabled");
  const hasLyrics = computed(() => lines.value.length > 0);

  watch(lineSize, (value) => localStorage.setItem("welkin-lyrics-size", String(value)));
  watch(translationSize, (value) => localStorage.setItem("welkin-lyrics-translation-size", String(value)));
  watch(lineSpacing, (value) => localStorage.setItem("welkin-lyrics-spacing", String(value)));

  function setSource(next: LyricsSource) {
    source.value = next;
    reset();
  }

  function setTranslate(next: boolean) {
    translate.value = next;
  }

  function load(next: LyricLine[]) {
    lines.value = next;
    status.value = "ready";
    error.value = "";
    activeIndex.value = -1;
  }

  async function loadForTrack(path: string | undefined) {
    reset();
    if (!path || !enabled.value) return;

    status.value = "loading";
    try {
      const content = await invoke<string>("read_track_lyrics", { path });
      const parsed = parseLyric(content, { applyOffset: true });
      if (parsed.lines.length === 0) {
        fail("empty");
        return;
      }
      load(parsed.lines);
    } catch (error) {
      fail(toAppError(error).code);
    }
  }

  function wordProgress(line: LyricLine, wordIndex: number, positionMs: number) {
    const word = line.words[wordIndex];
    if (!word) return 0;
    const next = line.words[wordIndex + 1];
    let end = word.endTime;
    if (next && end > next.startTime) end = next.startTime;
    if (end <= word.startTime) {
      end = next ? next.startTime : Math.max(line.endTime, word.startTime + 1);
    }
    const duration = Math.max(1, end - word.startTime);
    return Math.min(1, Math.max(0, (positionMs - word.startTime) / duration));
  }

  function sync(positionSeconds: number) {
    activeIndex.value = findLyricIndex(lines.value, positionSeconds * 1000);
  }

  function fail(message: string) {
    status.value = "error";
    error.value = message;
  }

  function reset() {
    status.value = "idle";
    error.value = "";
    lines.value = [];
    activeIndex.value = -1;
  }

  return {
    source,
    translate,
    lineSize,
    translationSize,
    lineSpacing,
    status,
    error,
    lines,
    activeIndex,
    enabled,
    hasLyrics,
    setSource,
    setTranslate,
    load,
    loadForTrack,
    fail,
    reset,
    sync,
    wordProgress,
  };
});
