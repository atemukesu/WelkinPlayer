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
  const useAmll = ref(localStorage.getItem("welkin-lyrics-amll") === "1");
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
  /** Guards against out-of-order lyric loads when switching tracks quickly. */
  let loadToken = 0;

  const enabled = computed(() => source.value !== "disabled");
  const hasLyrics = computed(() => lines.value.length > 0);

  watch(lineSize, (value) => localStorage.setItem("welkin-lyrics-size", String(value)));
  watch(translationSize, (value) => localStorage.setItem("welkin-lyrics-translation-size", String(value)));
  watch(lineSpacing, (value) => localStorage.setItem("welkin-lyrics-spacing", String(value)));
  watch(useAmll, (value) => localStorage.setItem("welkin-lyrics-amll", value ? "1" : "0"));

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

    const token = ++loadToken;
    status.value = "loading";

    // 1. Show the locally cached lyrics immediately (no network needed).
    let cached: string | null = null;
    try {
      cached = await invoke<string | null>("get_cached_lyrics", { path });
    } catch { cached = null; }
    let shown = false;
    if (cached && token === loadToken) {
      shown = applyContent(cached);
    }

    // 2. Ask the remote for the authoritative copy and update only if it differs.
    try {
      const content = await invoke<string>("read_track_lyrics", { path });
      if (token !== loadToken) return;
      if (content && content !== cached) {
        const applied = applyContent(content);
        if (!applied && !shown) fail("empty");
      } else if (!shown) {
        fail("empty");
      }
    } catch (error) {
      if (token !== loadToken) return;
      if (!shown) fail(toAppError(error).code);
    }
  }

  /** Parse lyric text and load it; returns false when there are no lines. */
  function applyContent(content: string): boolean {
    const parsed = parseLyric(content, { applyOffset: true });
    if (parsed.lines.length === 0) return false;
    load(parsed.lines);
    return true;
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
    useAmll,
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
