/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Reading a title for alphabetical ordering.
 *
 * Chinese hanzi become pinyin, Japanese kana become romaji, and everything
 * already latin passes through untouched. Readings are memoized in memory by
 * `lib/readings.ts` and persisted to the cache directory, so the conversion
 * only ever runs for a title the app has not seen before.
 *
 * Japanese *kanji* share the CJK ideograph block with Chinese, and a reading
 * accurate to the Japanese onyomi/kunyomi would need a full morphological
 * analyzer (a multi-megabyte dictionary). `pinyin-pro` gives the shared
 * ideographs a deterministic reading instead, so a title mixes into the latin
 * ordering rather than being dumped in the `#` bucket; whatever is left — pure
 * kana — is then romanized. Anything with neither script (Korean, Cyrillic,
 * symbols, …) yields no reading and lands in `#`.
 */

import { pinyin } from "pinyin-pro";
import { toRomaji } from "wanakana";

/** Han ideographs: CJK Unified, Extension A, and the compatibility block. */
const HAN = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/;
/** Kana: hiragana, katakana, kana extensions, and half-width katakana. */
const KANA = /[\u3040-\u309f\u30a0-\u30ff\u31f0-\u31ff\uff66-\uff9f]/;
const LATIN = /[a-z]/i;
/** Strips leading punctuation/whitespace so 「明日」 orders under M. */
const LEADING_NOISE = /^[^0-9a-z]+/i;

/** The `#` bucket used for titles with no latin reading. */
export const FALLBACK_BUCKET = "#";

/**
 * Romanize a title: hanzi → pinyin, kana → romaji, latin unchanged. Strings
 * with no han/kana are returned as-is so pure-latin titles stay cheap.
 */
export function romanize(value: string): string {
  if (!HAN.test(value) && !KANA.test(value)) return value;
  const converted = pinyin(value, {
    toneType: "none",
    type: "string",
    separator: "",
    nonZh: "consecutive",
  });
  return KANA.test(converted) ? toRomaji(converted) : converted;
}

/**
 * The normalized sort key for a title: its romanized reading with leading
 * punctuation and whitespace stripped, so a leading quote or bracket does not
 * decide the bucket.
 */
export function sortReading(value: string): string {
  return romanize(value).trim().replace(LEADING_NOISE, "");
}

/**
 * The bucketing weight of a reading: `0` for a latin initial (sorts among
 * A–Z), `1` for anything else (digits, empty, non-latin) so those group at the
 * end under `#`.
 */
export function bucketWeight(reading: string): number {
  return LATIN.test(reading.charAt(0)) ? 0 : 1;
}

/** The `A`–`Z` / `#` label a reading belongs to. */
export function bucketLabel(reading: string): string {
  const first = reading.charAt(0);
  return LATIN.test(first) ? first.toUpperCase() : FALLBACK_BUCKET;
}
