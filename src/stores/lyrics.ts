import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { detectFormat, findActiveLyricIndices, parseLyric, pickPrimaryIndex } from "lyric-kit";
import { invoke, toAppError } from "../api";
import {
  DEFAULT_AMLL_DISPLAY,
  DEFAULT_CLASSIC_DISPLAY,
  LYRIC_PROVIDERS,
  normalizeLyricProviders,
} from "../lib/preferences";
import type { LyricDisplaySettings, LyricProvider } from "../lib/preferences";
import { readLyricSource, tagLyric } from "../lib/lyricTag";
import { applyLyricOffset, readLyricOffset } from "../lib/lyricOffset";
import { narrowFullWidthDigits } from "../lib/lyricWidth";
import {
  LyricResolver,
  lyricFromAmll,
  lyricFromNetease,
  lyricFromQqBest,
} from "../lib/lyricSources";
import { lyricLog, since } from "../lib/lyricLog";
import { trackKey } from "../lib/sources";
import { useProfileStore } from "./profile";

export type LyricsStatus = "idle" | "loading" | "ready" | "error";

export type LyricLine = ReturnType<typeof parseLyric>["lines"][number];

export type { LyricDisplaySettings, LyricProvider };

/** Minimal track descriptor passed to the loader (title/artist drive online lookup). */
export interface LyricTrackInfo {
  path?: string;
  sourceId?: string;
  title?: string;
  artist?: string;
  album?: string;
}

const CLASSIC_KEY = "welkin-lyrics-classic";
const AMLL_KEY = "welkin-lyrics-amll-settings";
const ENABLED_KEY = "welkin-lyrics-enabled";
const PROVIDERS_KEY = "welkin-lyrics-providers";

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
    ruby: typeof data.ruby === "boolean" ? data.ruby : fallback.ruby,
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

/** Load the stored provider order, tolerating bad JSON. */
function loadProviders(): LyricProvider[] {
  const stored = localStorage.getItem(PROVIDERS_KEY);
  if (!stored) return normalizeLyricProviders(undefined);
  try {
    return normalizeLyricProviders(JSON.parse(stored));
  } catch {
    return normalizeLyricProviders(undefined);
  }
}

export const useLyricsStore = defineStore("lyrics", () => {
  const enabled = ref(localStorage.getItem(ENABLED_KEY) !== "0");
  const providers = ref<LyricProvider[]>(loadProviders());
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
  /** Every line whose time window contains the current position (BG + overlaps). */
  const activeIndices = ref<number[]>([]);
  /** Whether the track currently loaded has its lyrics disabled per-track. */
  const disabled = ref(false);
  /** Provider that supplied the currently loaded lyrics (null when unknown). */
  const source = ref<LyricProvider | null>(null);
  /** Detected format of the loaded lyrics (`ttml` / `qrc` / `lrc` / …). */
  const sourceFormat = ref("");
  /** Guards against out-of-order lyric loads when switching tracks quickly. */
  let loadToken = 0;

  const hasLyrics = computed(() => lines.value.length > 0);
  /** True when lyrics are intentionally off (global switch or this track), so
   * renderers should show nothing rather than a "no lyrics" message. */
  const suppressed = computed(() => !enabled.value || disabled.value);
  /** Typography for the renderer currently in use. */
  const display = computed(() => (useAmll.value ? amll.value : classic.value));

  watch(classic, (value) => localStorage.setItem(CLASSIC_KEY, JSON.stringify(value)), { deep: true });
  watch(amll, (value) => localStorage.setItem(AMLL_KEY, JSON.stringify(value)), { deep: true });
  watch(useAmll, (value) => localStorage.setItem("welkin-lyrics-amll", value ? "1" : "0"));
  watch(enabled, (value) => localStorage.setItem(ENABLED_KEY, value ? "1" : "0"));
  watch(providers, (value) => localStorage.setItem(PROVIDERS_KEY, JSON.stringify(value)), { deep: true });

  function setEnabled(next: boolean) {
    enabled.value = next;
    if (!next) reset();
  }

  function setProviders(next: LyricProvider[]) {
    providers.value = normalizeLyricProviders(next);
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
    activeIndices.value = [];
  }

  /**
   * Fetch lyrics for a track by walking the provider order and using the first
   * one that returns parsable content. The cached copy (from any provider) is
   * shown immediately while the lookup runs.
   *
   * `local` reads the same-named `.lrc` sidecar from WebDAV; every online hit
   * is written to the local cache and uploaded back to WebDAV as that sidecar,
   * so later plays resolve straight from `local`. AMLL is queried by the
   * NetEase/QQ platform ids resolved from one shared title/artist search.
   */
  async function loadForTrack(track: LyricTrackInfo | undefined) {
    reset();
    const path = track?.path;
    const trackSourceId = track?.sourceId;
    const key = trackKey(track ?? undefined);
    const isDisabled = key ? useProfileStore().isLyricsDisabled(key) : false;
    disabled.value = isDisabled;
    if (!path || !enabled.value || isDisabled) {
      lyricLog("info", "skip load", { path, enabled: enabled.value, disabled: isDisabled });
      return;
    }

    const token = ++loadToken;
    const started = performance.now();
    status.value = "loading";
    lyricLog("info", "load start", {
      path,
      title: track?.title,
      artist: track?.artist,
      providers: [...providers.value],
    });

    // 1. Show the locally cached lyrics immediately (no network needed).
    let shown = false;
    try {
      const cached = await invoke<string | null>("get_cached_lyrics", { sourceId: trackSourceId, path });
      if (token !== loadToken) return;
      if (cached) {
        shown = applyContent(cached);
        if (shown) rememberSource(cached);
        lyricLog("info", "cache hit", `${cached.length} chars`);
      } else {
        lyricLog("info", "cache miss");
      }
    } catch (error) {
      lyricLog("warn", "cache read failed", error);
    }

    // 2. Walk the provider order; the first hit wins and is persisted.
    const query = {
      title: (track?.title ?? "").trim(),
      artist: (track?.artist ?? "").trim(),
      album: (track?.album ?? "").trim(),
    };
    const resolver = new LyricResolver(query);

    let lastError = "";
    for (const provider of providers.value) {
      if (token !== loadToken) return;
      lyricLog("info", `try provider: ${provider}`);
      try {
        let content: string | null = null;
        let sourceId: string | undefined;
        if (provider === "local") {
          content = await readLocalLyrics(path, trackSourceId);
        } else if (provider === "amll") {
          await resolver.netease();
          await resolver.qq();
          const hit = await lyricFromAmll(resolver.cached, query);
          if (hit) {
            content = hit.content;
            sourceId = hit.sourceId;
          }
        } else if (provider === "netease") {
          const ids = await resolver.netease();
          if (ids.netease) {
            content = await lyricFromNetease(ids.netease);
            sourceId = `ncm/${ids.netease}`;
          }
        } else if (provider === "qq") {
          const hit = await lyricFromQqBest(await resolver.qq());
          if (hit) {
            content = hit.content;
            sourceId = hit.sourceId;
          }
        }
        if (token !== loadToken) return;
        if (content && applyContent(content)) {
          rememberSource(content, provider);
          lyricLog(
            "info",
            `provider ${provider} hit`,
            `${content.length} chars, ${lines.value.length} lines, format=${
              sourceFormat.value || "?"
            } (${since(started)})`,
          );
          // Online lyrics are tagged, cached and mirrored to WebDAV; a `local`
          // hit is already the sidecar itself.
          if (provider !== "local") void persistLyrics(path, trackSourceId, content, provider, sourceId);
          return;
        }
        lyricLog("info", `provider ${provider} miss (unparsable or empty)`);
      } catch (error) {
        if (token !== loadToken) return;
        lastError = toAppError(error).code;
        lyricLog("error", `provider ${provider} failed`, error);
      }
    }

    if (token !== loadToken) return;
    // A cached copy is still a valid result even if the network lookup failed.
    if (!shown) {
      lyricLog("error", "no lyrics found", `${lastError || "empty"} (${since(started)})`);
      fail(lastError || "empty");
    } else {
      lyricLog("warn", "no provider hit; keeping the cached copy", since(started));
    }
  }

  /** Read the same-named `.lrc` sidecar from the track's source. */
  async function readLocalLyrics(path: string, trackSourceId?: string): Promise<string | null> {
    try {
      const content = await invoke<string>("read_track_lyrics", { sourceId: trackSourceId, path });
      const ok = Boolean(content && content.trim());
      lyricLog(
        ok ? "info" : "warn",
        "local sidecar",
        ok ? `${content.length} chars` : "found but blank",
      );
      return ok ? content : null;
    } catch (error) {
      lyricLog("warn", "local sidecar unavailable", error);
      return null;
    }
  }

  /**
   * Tag the lyric with its origin, cache it locally and mirror it to WebDAV as
   * a same-named sidecar; failures are non-fatal.
   */
  async function persistLyrics(
    path: string,
    trackSourceId: string | undefined,
    content: string,
    provider: LyricProvider,
    sourceId?: string,
  ) {
    // Stored copies get the half-width form too, so a later cache hit (or the
    // WebDAV sidecar) is already clean and the editor never sees １６４ again.
    const tagged = tagLyric(narrowFullWidthDigits(content), { source: provider, sourceId });
    try {
      const result = await invoke<{ uploaded: boolean; uploadError: string | null }>(
        "save_track_lyrics",
        { sourceId: trackSourceId, path, content: tagged },
      );
      if (result.uploadError) {
        lyricLog(
          "warn",
          `persisted from ${provider}`,
          `${tagged.length} chars -> local cache only (WebDAV: ${result.uploadError})`,
        );
      } else if (!result.uploaded) {
        lyricLog(
          "info",
          `persisted from ${provider}`,
          `${tagged.length} chars -> local cache only (WebDAV not configured)`,
        );
      } else {
        lyricLog(
          "info",
          `persisted from ${provider}`,
          `${tagged.length} chars -> local cache + WebDAV sidecar`,
        );
      }
    } catch (error) {
      lyricLog("error", `persist failed from ${provider}`, error);
    }
  }

  function isProvider(value: string | null | undefined): value is LyricProvider {
    return typeof value === "string" && (LYRIC_PROVIDERS as string[]).includes(value);
  }

  /** Remember where the loaded lyrics came from (tagged file first, provider as fallback). */
  function rememberSource(content: string, provider?: LyricProvider) {
    sourceFormat.value = detectFormat(content);
    const tagged = readLyricSource(content);
    source.value = isProvider(tagged?.source) ? tagged.source : provider ?? null;
  }

  /** Parse lyric text and load it; returns false when there are no lines. */
  function applyContent(content: string): boolean {
    // Normalise width once, so display, offset and format detection all see
    // the same half-width digits the renderer will draw.
    const normalized = narrowFullWidthDigits(content);
    const parsed = parseLyric(normalized, { applyOffset: true, extractMetadata: true });
    // `lyric-kit` applies `[offset]` for LRC-family payloads, but keeps the TTML
    // offset tag as inert metadata — shift it here so both formats behave alike.
    if (detectFormat(normalized) === "ttml") {
      applyLyricOffset(parsed.lines, readLyricOffset(normalized));
    }
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

  /** Replace the active set only when it really changed (avoids a re-render every tick). */
  function setActiveIndices(next: number[]) {
    const current = activeIndices.value;
    if (current.length === next.length && current.every((value, index) => value === next[index])) return;
    activeIndices.value = next;
  }

  function sync(positionSeconds: number) {
    const positionMs = positionSeconds * 1000;
    activeIndex.value = pickPrimaryIndex(lines.value, positionMs);
    setActiveIndices(findActiveLyricIndices(lines.value, positionMs));
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
    activeIndices.value = [];
    source.value = null;
    sourceFormat.value = "";
    disabled.value = false;
  }

  return {
    enabled,
    providers,
    translate: computed(() => display.value.translate),
    useAmll,
    classic,
    amll,
    display,
    status,
    error,
    lines,
    activeIndex,
    activeIndices,
    source,
    sourceFormat,
    hasLyrics,
    suppressed,
    disabled,
    setEnabled,
    setProviders,
    setTranslate,
    load,
    loadForTrack,
    fail,
    reset,
    sync,
    wordProgress,
  };
});
