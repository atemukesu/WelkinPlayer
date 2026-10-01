//! Width normalisation for whole lyric payloads.

const FULLWIDTH_DIGITS = /[０-９]/g;

/**
 * Rewrite every full-width digit in a lyric payload to its half-width form.
 *
 * Timing tags, TTML markup and metadata keys are all ASCII, so a blanket
 * replace only ever touches the lyric body (and any metadata *values*, where
 * half-width is equally fine). `１６４` becomes `164`, which also keeps the
 * `[kana:]` pairing in `qrcKana.ts` looking at a single digit run.
 */
export function narrowFullWidthDigits(text: string): string {
  return text.replace(FULLWIDTH_DIGITS, (digit) => String.fromCharCode(digit.charCodeAt(0) - 0xfee0));
}
