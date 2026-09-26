//! QQ kana (注音) pairing for QRC lyrics.
//!
//! QQ ships a single `[kana:…]` meta line that lists the reading of every base
//! character of the whole song, in order, as units written `N<reading>` where
//! `N` is how many base characters the reading covers (`2きょう` for 今日).
//! Pairing is purely positional — the readings carry no reference to the text
//! they belong to — so the base list has to be counted exactly the way the tag
//! author counted it, which is: **one base per Han character, plus one base per
//! run of digits**. That second half is the part `lyric-kit` gets wrong: it
//! counts every digit character separately, so the credit lines (`词：164`)
//! inject two extra slots each and every reading after them is shifted.
//!
//! This module therefore strips the `[kana:]` line before `parseQRC` (so
//! `lyric-kit` never sees it) and re-applies the readings itself, using the
//! corrected base list. Kept dependency-free apart from type imports so it can
//! be exercised from plain Node.

import type { LyricLine, LyricWord } from "lyric-kit";

/** Characters that can carry a reading: CJK, the iteration marks 々/〆, digits. */
const KANA_BASE = /[\u4e00-\u9fff\u3400-\u4dbf\u3005\u30060-9]/;

/** Matches the whole `[kana:…]` meta line. */
const KANA_TAG = /^\s*\[kana:(.*)\]\s*$/i;

/** One `N<reading>` unit of the kana tag. */
interface KanaUnit {
  /** How many base characters the reading covers. */
  kanjiCount: number;
  /** The reading itself; empty for the placeholder units that stand for digits. */
  kanaText: string;
  /** Per-syllable timings, when the tag carries them. */
  spans?: Array<{ word: string; startTime: number; endTime: number }>;
}

/** A base character that a reading can attach to. */
interface KanaTarget {
  word: LyricWord;
  /** Index of the character inside `word.word`. */
  charIndex: number;
}

/**
 * Split a QRC payload into the text without its `[kana:]` line and that line's
 * raw reading list. Returns an empty `tag` when the payload has none.
 */
export function extractKanaTag(text: string): { text: string; tag: string } {
  let tag = "";
  const kept: string[] = [];
  for (const line of text.split(/\r?\n/)) {
    const match = KANA_TAG.exec(line);
    if (match && !tag) {
      tag = match[1];
      continue;
    }
    kept.push(line);
  }
  return { text: kept.join("\n"), tag };
}

/** Parse the raw reading list into units. */
function parseKanaUnits(tag: string): KanaUnit[] {
  const units: KanaUnit[] = [];
  let kanjiCount = 0;
  let kanaText = "";
  let spans: KanaUnit["spans"];

  const flush = () => {
    if (kanjiCount <= 0) return;
    units.push({ kanjiCount, kanaText, spans: spans?.length ? spans : undefined });
    kanaText = "";
    spans = undefined;
  };

  for (let index = 0; index < tag.length; index++) {
    const char = tag[index];

    if (char >= "1" && char <= "9") {
      flush();
      kanjiCount = Number(char);
      continue;
    }

    if (char === "(") {
      // `N<reading>(start,duration)` gives one syllable its own timing; the
      // syllable is the character immediately before the parenthesis.
      const close = tag.indexOf(")", index);
      if (close < 0) {
        kanaText += char;
        continue;
      }
      const [start, duration] = tag.slice(index + 1, close).split(",").map(Number);
      if (kanaText.length > 0 && Number.isFinite(start) && Number.isFinite(duration)) {
        (spans ??= []).push({
          word: kanaText[kanaText.length - 1],
          startTime: start,
          endTime: start + duration,
        });
      }
      index = close;
      continue;
    }

    kanaText += char;
  }

  flush();
  return units;
}

/**
 * Collect the base characters the readings are paired against, in order.
 *
 * A run of consecutive digits counts as **one** base: the tag contains an empty
 * placeholder unit for it (e.g. `…1し11きょく…` — the bare `1` is the reading
 * slot for `164`). Counting each digit separately is what shifts the readings.
 */
function collectTargets(lines: LyricLine[]): KanaTarget[] {
  const targets: KanaTarget[] = [];
  for (const line of lines) {
    for (const word of line.words) {
      const text = word.word;
      for (let index = 0; index < text.length; index++) {
        const char = text[index];
        if (char >= "0" && char <= "9") {
          targets.push({ word, charIndex: index });
          while (index + 1 < text.length && text[index + 1] >= "0" && text[index + 1] <= "9") index++;
          continue;
        }
        if (KANA_BASE.test(char)) targets.push({ word, charIndex: index });
      }
    }
  }
  return targets;
}

/**
 * Attach the readings to `line.words[].ruby`, in place.
 *
 * Alignment follows the tag's own rules: a unit consumes `kanjiCount` base
 * characters and its reading is hung on the first of them; a single-character
 * unit gets its timing narrowed to that character's share of the word.
 */
export function applyKanaToLines(lines: LyricLine[], tag: string): void {
  if (!tag) return;
  const units = parseKanaUnits(tag);
  if (units.length === 0) return;

  const targets = collectTargets(lines);
  if (targets.length === 0) return;

  let pointer = 0;
  for (const unit of units) {
    if (pointer >= targets.length) break;
    const count = Math.min(unit.kanjiCount, targets.length - pointer);
    const group = targets.slice(pointer, pointer + count);
    pointer += count;
    // The placeholder units that stand for a digit run carry no reading.
    if (!unit.kanaText) continue;

    if (unit.spans && unit.spans.length > 0) {
      const primary = group[0].word;
      primary.ruby = [...(primary.ruby ?? []), ...unit.spans];
      continue;
    }

    if (group.length === 1) {
      const { word, charIndex } = group[0];
      let { startTime, endTime } = word;
      const total = word.word.length;
      if (total > 1 && endTime > startTime) {
        const charDuration = (endTime - startTime) / total;
        startTime = Math.round(word.startTime + charIndex * charDuration);
        endTime = Math.round(startTime + charDuration);
      }
      word.ruby = [...(word.ruby ?? []), { word: unit.kanaText, startTime, endTime }];
      continue;
    }

    const first = group[0].word;
    const last = group[group.length - 1].word;
    first.ruby = [
      ...(first.ruby ?? []),
      { word: unit.kanaText, startTime: first.startTime, endTime: Math.max(last.endTime, first.startTime) },
    ];
  }
}
