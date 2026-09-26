//! Turn QQ's word-by-word QRC track into TTML, optionally merging in the
//! plaintext translation track that ships alongside it.
//!
//! QQ's `[kana:]` reading line is applied here instead of by `lyric-kit`, whose
//! positional pairing is thrown off by the digits in the credit lines — see
//! `qrcKana.ts`. Kept in its own module so it can be exercised from plain Node:
//! `qrcKana.ts` carries no relative imports, while the imports below are
//! extensionless (bundler-only), so Node tests import `qrcKana.ts` directly.

import { pairTranslation, parseLRC, parseQRC, toTTML } from "lyric-kit";

import { applyKanaToLines, extractKanaTag } from "./qrcKana";

/** Watermarks/placeholders QQ puts in its translation track. */
const TRANSLATION_NOISE = /(翻译作品|著作权)/;

function lineText(line: { words: Array<{ word: string }> }): string {
  return line.words.map((word) => word.word).join("").trim();
}

/**
 * Serialize QQ's word-by-word QRC track to TTML, with its kana readings and —
 * when `translationLrc` is given — its translation track merged in.
 *
 * `lyric-kit` can only carry word timings **and** translations together in
 * TTML, so the paired lines are serialized back to a TTML string.
 */
export function qrcToTtml(qrcText: string, translationLrc?: string): string {
  const { text: stripped, tag } = extractKanaTag(qrcText);
  const main = parseQRC(stripped);
  applyKanaToLines(main.lines, tag);

  if (translationLrc) {
    const translation = parseLRC(translationLrc);
    const useful = translation.lines.filter((line) => {
      const text = lineText(line);
      return text.length > 0 && text !== "//" && !TRANSLATION_NOISE.test(text);
    });
    pairTranslation(main.lines, useful, "translatedLyric");
  }

  return toTTML(main.lines);
}
