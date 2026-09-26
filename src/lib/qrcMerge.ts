//! Merge QQ's word-by-word QRC track with its plaintext translation track.
//!
//! Kept in its own module (importing nothing but `lyric-kit`) so the merge can
//! be validated from plain Node, where the extensionless project imports used
//! by `lyricSources.ts` do not resolve.

import { pairTranslation, parseLRC, parseQRC, toTTML } from "lyric-kit";

/** Watermarks/placeholders QQ puts in its translation track. */
const TRANSLATION_NOISE = /(翻译作品|著作权)/;

function lineText(line: { words: Array<{ word: string }> }): string {
  return line.words.map((word) => word.word).join("").trim();
}

/**
 * Pair QQ's word-by-word QRC track with its plaintext translation track.
 *
 * `lyric-kit` can only carry word timings **and** translations together in
 * TTML, so the paired lines are serialized back to a TTML string.
 */
export function mergeQrcTranslation(qrcText: string, translationLrc: string): string {
  const main = parseQRC(qrcText);
  const translation = parseLRC(translationLrc);
  const useful = translation.lines.filter((line) => {
    const text = lineText(line);
    return text.length > 0 && text !== "//" && !TRANSLATION_NOISE.test(text);
  });
  pairTranslation(main.lines, useful, "translatedLyric");
  return toTTML(main.lines);
}
