import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { findLyricIndex, parseLyric } from "lyric-kit";
import { invoke, toAppError } from "../api";
import {
  DEFAULT_AMLL_DISPLAY,
  DEFAULT_CLASSIC_DISPLAY,
} from "../lib/profile";
import type { LyricDisplaySettings } from "../lib/profile";

export type LyricsSource = "local" | "disabled";
export type LyricsStatus = "idle" | "loading" | "ready" | "error";

export type LyricLine = ReturnType<typeof parseLyric>["lines"][number];

export type { LyricDisplaySettings };

const CLASSIC_KEY = "welkin-lyrics-classic";
const AMLL_KEY = "welkin-lyrics-amll-settings";

function clampNumber(value: unknown, min: number, max: number, fallback: number): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.min(max, Math.max(min, numeric));
}

/** Snap a weight to the nearest 100 between 100 and 900. */
function clampWeight(value: unknown, fallback: number): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.min(900, Math.max(100, Math.round(numeric / 100) * 100));
}

function normalizeSettings(raw: unknown, fallback: LyricDisplaySettings): LyricDisplaySettings {
  const data = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  return {
    lineSize: clampNumber(data.lineSize, 16, 48, fallback.lineSize),
    translationSize: clampNumber(data.translationSize, 12, 32, fallback.translationSize),
    lineSpacing: clampNumber(data.lineSpacing, 8, 36, fallback.lineSpacing),
    translate: typeof data.translate === "boolean" ? data.translate : fallback.translate,
    fontWeight: clampWeight(data.fontWeight, fallback.fontWeight),
    fontFamilies: Array.isArray(data.fontFamilies)
      ? data.fontFamilies.filter((item): item is string => typeof item === "string")
      : [...fallback.fontFamilies],
  };
}

function readLegacySetting(key: string, min: number, max: number): number | undefined {
  const value = Number(localStorage.getItem(key));
  if (!Number.isFinite(value) || value < min || value > max) return undefined;
  return value;
}

/** Load one renderer's settings from localStorage, tolerating bad JSON. */
function loadSettings(key: string, fallback: LyricDisplaySettings): LyricDisplaySettings {
  const stored = localStorage.getItem(key);
  if (!stored) return { ...fallback };
  try {
    return normalizeSettings(JSON.parse(stored), fallback);
  } catch {
    return { ...fallback };
  }
}

export const useLyricsStore = defineStore("lyrics", () => {
  const source = ref<LyricsSource>("local");
  const useAmll = ref(localStorage.getItem("welkin-lyrics-amll") === "1");

  // Migrate the pre-split keys into the new per-mode defaults on first load.
  const legacySize = readLegacySetting("welkin-lyrics-size", 16, 48);
  const legacyTranslationSize = readLegacySetting("welkin-lyrics-translation-size", 12, 32);
  const legacyLineSpacing = readLegacySetting("welkin-lyrics-spacing", 8, 36);

  const classic = ref<LyricDisplaySettings>(
    loadSettings(CLASSIC_KEY, {
      ...DEFAULT_CLASSIC_DISPLAY,
      ...(legacySize !== undefined ? { lineSize: legacySize } : {}),
      ...(legacyTranslationSize !== undefined ? { translationSize: legacyTranslationSize } : {}),
      ...(legacyLineSpacing !== undefined ? { lineSpacing: legacyLineSpacing } : {}),
    }),
  );
  const amll = ref<LyricDisplaySettings>(
    loadSettings(AMLL_KEY, {
      ...DEFAULT_AMLL_DISPLAY,
      ...(legacySize !== undefined ? { lineSize: legacySize } : {}),
      ...(legacyTranslationSize !== undefined ? { translationSize: legacyTranslationSize } : {}),
      ...(legacyLineSpacing !== undefined ? { lineSpacing: legacyLineSpacing } : {}),
    }),
  );

  const status = ref<LyricsStatus>("idle");
  const error = ref("");
  const lines = ref<LyricLine[]>([]);
  const activeIndex = ref(-1);
  /** Guards against out-of-order lyric loads when switching tracks quickly. */
  let loadToken = 0;

  const enabled = computed(() => source.value !== "disabled");
  const hasLyrics = computed(() => lines.value.length > 0);
  /** Typography for the renderer currently in use. */
  const display = computed(() => (useAmll.value ? amll.value : classic.value));

  watch(classic, (value) => localStorage.setItem(CLASSIC_KEY, JSON.stringify(value)), { deep: true });
  watch(amll, (value) => localStorage.setItem(AMLL_KEY, JSON.stringify(value)), { deep: true });
  watch(useAmll, (value) => localStorage.setItem("welkin-lyrics-amll", value ? "1" : "0"));

  function setSource(next: LyricsSource) {
    source.value = next;
    reset();
  }

  function setTranslate(next: boolean) {
    display.value.translate = next;
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
    translate: computed(() => display.value.translate),
    useAmll,
    classic,
    amll,
    display,
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
