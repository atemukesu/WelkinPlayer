import { computed, ref } from "vue";
import { invoke } from "../api";

/**
 * Families that must stay as bare CSS keywords instead of being quoted, so
 * `system-ui` keeps meaning the generic family rather than a literal font
 * named "system-ui".
 */
const GENERIC_FAMILIES = new Set([
  "serif",
  "sans-serif",
  "monospace",
  "cursive",
  "fantasy",
  "system-ui",
  "ui-serif",
  "ui-sans-serif",
  "ui-monospace",
  "ui-rounded",
  "emoji",
  "math",
  "fangsong",
]);

/** Families always offered even though `fontdb` only reports installed files. */
const ALWAYS_AVAILABLE = [
  "system-ui",
  "sans-serif",
  "serif",
  "monospace",
  "Noto Sans SC Variable",
];

/** Render an ordered family list as a CSS `font-family` value. */
export function cssFontFamily(families: readonly string[]): string | undefined {
  const stack = families
    .map((family) => family.trim())
    .filter(Boolean)
    .map((family) => {
      if (GENERIC_FAMILIES.has(family.toLowerCase())) return family;
      return `"${family.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
    });
  return stack.length > 0 ? stack.join(", ") : undefined;
}

const systemFonts = ref<string[]>([]);
let loadStarted = false;

/**
 * Lazily load the installed font families from Rust once per session, and
 * expose them alongside the always-available generic families.
 */
export function useSystemFonts() {
  if (!loadStarted) {
    loadStarted = true;
    invoke<string[]>("list_system_fonts")
      .then((fonts) => {
        if (Array.isArray(fonts) && fonts.length > 0) systemFonts.value = fonts;
      })
      .catch((error) => {
        console.warn("[welkin] failed to list system fonts", error);
      });
  }

  const allFonts = computed(() => {
    const seen = new Set<string>();
    const merged: string[] = [];
    for (const family of [...ALWAYS_AVAILABLE, ...systemFonts.value]) {
      const key = family.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      merged.push(family);
    }
    return merged;
  });

  return { systemFonts, allFonts };
}
