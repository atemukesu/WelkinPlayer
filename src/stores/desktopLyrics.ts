import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import {
  DEFAULT_DESKTOP_LYRIC,
  type DesktopLyricAlign,
  type DesktopLyricSettings,
  type DesktopLyricWheelAction,
} from "../lib/preferences";
import { isAndroid } from "../lib/desktopLyric";

const SETTINGS_KEY = "welkin-desktop-lyric";

/** Clamp a stored numeric field into range, falling back when unparsable. */
function clampNumber(value: unknown, min: number, max: number, fallback: number): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.min(max, Math.max(min, numeric));
}

function clampWeight(value: unknown, fallback: number): number {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.min(900, Math.max(100, Math.round(numeric / 100) * 100));
}

function pickEnum<T extends string>(value: unknown, allowed: readonly T[], fallback: T): T {
  return typeof value === "string" && (allowed as readonly string[]).includes(value) ? (value as T) : fallback;
}

function asColor(value: unknown, fallback: string): string {
  return typeof value === "string" && value.trim() ? value.trim() : fallback;
}

/** Every boolean field, so the helper below can stay strictly typed. */
type BooleanKey =
  | "enabled"
  | "locked"
  | "alwaysOnTop"
  | "skipTaskbar"
  | "translation"
  | "karaoke"
  | "stroke"
  | "hideOnPause"
  | "hideWhenNoLyrics";

/** Merge an untrusted stored blob over the defaults, clamping every field. */
export function normalizeDesktopLyric(raw: unknown): DesktopLyricSettings {
  const data = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const fallback = DEFAULT_DESKTOP_LYRIC;
  const bool = (key: BooleanKey): boolean =>
    typeof data[key] === "boolean" ? (data[key] as boolean) : fallback[key];
  return {
    enabled: bool("enabled"),
    locked: bool("locked"),
    alwaysOnTop: bool("alwaysOnTop"),
    skipTaskbar: bool("skipTaskbar"),
    translation: bool("translation"),
    karaoke: bool("karaoke"),
    contextLines: clampNumber(data.contextLines, 0, 5, fallback.contextLines),
    align: pickEnum<DesktopLyricAlign>(data.align, ["left", "center", "right"], fallback.align),
    fontSize: clampNumber(data.fontSize, 14, 96, fallback.fontSize),
    translationSize: clampNumber(data.translationSize, 10, 56, fallback.translationSize),
    lineSpacing: clampNumber(data.lineSpacing, 0, 64, fallback.lineSpacing),
    fontWeight: clampWeight(data.fontWeight, fallback.fontWeight),
    fontFamilies: Array.isArray(data.fontFamilies)
      ? data.fontFamilies.filter((item): item is string => typeof item === "string")
      : [...fallback.fontFamilies],
    textColor: asColor(data.textColor, fallback.textColor),
    activeColor: asColor(data.activeColor, fallback.activeColor),
    translationColor: asColor(data.translationColor, fallback.translationColor),
    opacity: clampNumber(data.opacity, 10, 100, fallback.opacity),
    stroke: bool("stroke"),
    paddingX: clampNumber(data.paddingX, 0, 80, fallback.paddingX),
    paddingY: clampNumber(data.paddingY, 0, 80, fallback.paddingY),
    hideOnPause: bool("hideOnPause"),
    hideWhenNoLyrics: bool("hideWhenNoLyrics"),
    autoHideMs: clampNumber(data.autoHideMs, 0, 300_000, fallback.autoHideMs),
    wheelAction: pickEnum<DesktopLyricWheelAction>(data.wheelAction, ["none", "fontSize", "opacity"], fallback.wheelAction),
  };
}

function loadSettings(): DesktopLyricSettings {
  const stored = localStorage.getItem(SETTINGS_KEY);
  if (!stored) return { ...DEFAULT_DESKTOP_LYRIC };
  try {
    return normalizeDesktopLyric(JSON.parse(stored));
  } catch {
    return { ...DEFAULT_DESKTOP_LYRIC };
  }
}

export const useDesktopLyricsStore = defineStore("desktopLyrics", () => {
  const settings = ref<DesktopLyricSettings>(loadSettings());
  /** Whether this backend can host a floating layer at all. */
  const supported = ref(true);
  /** Android only: whether SYSTEM_ALERT_WINDOW has been granted. */
  const permissionGranted = ref(true);
  /** Whether a floating renderer is currently live. */
  const active = ref(false);
  /** Last backend error code, for the settings panel. */
  const error = ref("");
  const busy = ref(false);

  watch(settings, (value) => localStorage.setItem(SETTINGS_KEY, JSON.stringify(value)), { deep: true });

  /** Platform quirks surfaced in the UI. */
  const platform = computed(() => (isAndroid() ? "android" : "desktop"));

  function update(patch: Partial<DesktopLyricSettings>) {
    settings.value = normalizeDesktopLyric({ ...settings.value, ...patch });
  }

  return {
    settings,
    supported,
    permissionGranted,
    active,
    error,
    busy,
    platform,
    update,
  };
});
