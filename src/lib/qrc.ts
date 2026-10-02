/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

//! QQ Music QRC decryption (word-by-word lyrics).
//!
//! `lyric_download.fcg?lrctype=4` returns a hex payload that QQ encrypts with a
//! deliberately "broken" DES variant applied three times (decrypt → encrypt →
//! decrypt) and then zlib-compresses. A standard DES/3DES implementation cannot
//! decrypt it: two S-boxes contain duplicated entries and the key schedule masks
//! with `0xfffffff0` instead of `0x0fffffff`.
//!
//! The bit-exact algorithm below is ported from the MIT-licensed C++ reference
//! in `qq49371114/lx-music-api-server` (`deps/pyqdes/des.cpp`, itself Brad
//! Conte's public-domain DES). Nothing here is GPL.
//!
//! Decompression is a small self-contained DEFLATE (RFC 1950/1951) decoder.
//! It deliberately avoids `DecompressionStream` + `new Response(stream)`, which
//! faults inside the Tauri WebView2 runtime (`TypeError: Failed to fetch`), and
//! avoids adding a Rust zlib dependency.

const KEY1 = [0x21, 0x40, 0x23, 0x29, 0x28, 0x4e, 0x48, 0x4c, 0x69, 0x75, 0x79, 0x2a, 0x24, 0x25, 0x5e, 0x26];
const KEY2 = [0x31, 0x32, 0x33, 0x5a, 0x58, 0x43, 0x21, 0x40, 0x23, 0x29, 0x28, 0x2a, 0x24, 0x25, 0x5e, 0x26];
const KEY3 = [0x21, 0x40, 0x23, 0x29, 0x28, 0x2a, 0x24, 0x25, 0x5e, 0x26, 0x61, 0x62, 0x63, 0x44, 0x45, 0x46];

const SBOX1 = [
  14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7,
  0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12, 11, 9, 5, 3, 8,
  4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0,
  15, 12, 8, 2, 4, 9, 1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
];
// NOTE: row 2 holds two 15s — this corruption is intentional and part of QQ's DES.
const SBOX2 = [
  15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10,
  3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1, 10, 6, 9, 11, 5,
  0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15,
  13, 8, 10, 1, 3, 15, 4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
];
const SBOX3 = [
  10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8,
  13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5, 14, 12, 11, 15, 1,
  13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7,
  1, 10, 13, 0, 6, 9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
];
// Row 3 holds two 10s — same intentional corruption.
const SBOX4 = [
  7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15,
  13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2, 12, 1, 10, 14, 9,
  10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4,
  3, 15, 0, 6, 10, 10, 13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
];
const SBOX5 = [
  2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9,
  14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15, 10, 3, 9, 8, 6,
  4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14,
  11, 8, 12, 7, 1, 14, 2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
];
const SBOX6 = [
  12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11,
  10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13, 14, 0, 11, 3, 8,
  9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6,
  4, 3, 2, 12, 9, 5, 15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
];
const SBOX7 = [
  4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1,
  13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5, 12, 2, 15, 8, 6,
  1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2,
  6, 11, 13, 8, 1, 4, 10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
];
const SBOX8 = [
  13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7,
  1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6, 11, 0, 14, 9, 2,
  7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8,
  2, 1, 14, 7, 4, 10, 8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
];

/** Standard DES initial permutation, as 0-based source bit indices. */
const IP_TABLE = [
  57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3,
  61, 53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
  56, 48, 40, 32, 24, 16, 8, 0, 58, 50, 42, 34, 26, 18, 10, 2,
  60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6,
];

const KEY_PERM_C = [
  56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17,
  9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35,
];
const KEY_PERM_D = [
  62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21,
  13, 5, 60, 52, 44, 36, 28, 20, 12, 4, 27, 19, 11, 3,
];
const KEY_COMPRESSION = [
  13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9,
  22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1,
  40, 51, 30, 36, 46, 54, 29, 39, 50, 44, 32, 47,
  43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31,
];
const KEY_RND_SHIFT = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];

/** C's `BITNUM(a, b, c)` — bit `b` of an 8-byte block, shifted left by `c`. */
function bitnum(a: ArrayLike<number>, b: number, c: number): number {
  return (((a[((b >> 5) << 2) + 3 - ((b % 32) >> 3)] >>> (7 - (b % 8))) & 1) << c) >>> 0;
}

/** C's `BITNUMINTR(a, b, c)` — bit `b` of a 32-bit word, shifted left by `c`. */
function bitnumIntr(a: number, b: number, c: number): number {
  return (((a >>> (31 - b)) & 1) << c) >>> 0;
}

/** C's `BITNUMINTL(a, b, c)` — carry the leftmost bit after shifting left by `b`. */
function bitnumIntl(a: number, b: number, c: number): number {
  return (((a << b) & 0x80000000) >>> c) >>> 0;
}

/** C's `SBOXBIT(a)` — swap the outer bits with the row selector bits. */
function sboxBit(a: number): number {
  return ((a & 0x20) | ((a & 0x1f) >> 1) | ((a & 0x01) << 4)) >>> 0;
}

function initialPermutation(input: ArrayLike<number>): [number, number] {
  let lo = 0;
  let hi = 0;
  for (let k = 0; k < 64; k += 1) {
    const bit = bitnum(input, IP_TABLE[k], 0);
    if (k < 32) lo |= bit << (31 - k);
    else hi |= bit << (63 - k);
  }
  return [lo >>> 0, hi >>> 0];
}

/** Ported literally from the C reference (bit order is deliberately unusual). */
function inverseInitialPermutation(state: [number, number], out: Uint8Array): void {
  const [s0, s1] = state;
  out[3] =
    bitnumIntr(s1, 7, 7) | bitnumIntr(s0, 7, 6) | bitnumIntr(s1, 15, 5) | bitnumIntr(s0, 15, 4) |
    bitnumIntr(s1, 23, 3) | bitnumIntr(s0, 23, 2) | bitnumIntr(s1, 31, 1) | bitnumIntr(s0, 31, 0);
  out[2] =
    bitnumIntr(s1, 6, 7) | bitnumIntr(s0, 6, 6) | bitnumIntr(s1, 14, 5) | bitnumIntr(s0, 14, 4) |
    bitnumIntr(s1, 22, 3) | bitnumIntr(s0, 22, 2) | bitnumIntr(s1, 30, 1) | bitnumIntr(s0, 30, 0);
  out[1] =
    bitnumIntr(s1, 5, 7) | bitnumIntr(s0, 5, 6) | bitnumIntr(s1, 13, 5) | bitnumIntr(s0, 13, 4) |
    bitnumIntr(s1, 21, 3) | bitnumIntr(s0, 21, 2) | bitnumIntr(s1, 29, 1) | bitnumIntr(s0, 29, 0);
  out[0] =
    bitnumIntr(s1, 4, 7) | bitnumIntr(s0, 4, 6) | bitnumIntr(s1, 12, 5) | bitnumIntr(s0, 12, 4) |
    bitnumIntr(s1, 20, 3) | bitnumIntr(s0, 20, 2) | bitnumIntr(s1, 28, 1) | bitnumIntr(s0, 28, 0);
  out[7] =
    bitnumIntr(s1, 3, 7) | bitnumIntr(s0, 3, 6) | bitnumIntr(s1, 11, 5) | bitnumIntr(s0, 11, 4) |
    bitnumIntr(s1, 19, 3) | bitnumIntr(s0, 19, 2) | bitnumIntr(s1, 27, 1) | bitnumIntr(s0, 27, 0);
  out[6] =
    bitnumIntr(s1, 2, 7) | bitnumIntr(s0, 2, 6) | bitnumIntr(s1, 10, 5) | bitnumIntr(s0, 10, 4) |
    bitnumIntr(s1, 18, 3) | bitnumIntr(s0, 18, 2) | bitnumIntr(s1, 26, 1) | bitnumIntr(s0, 26, 0);
  out[5] =
    bitnumIntr(s1, 1, 7) | bitnumIntr(s0, 1, 6) | bitnumIntr(s1, 9, 5) | bitnumIntr(s0, 9, 4) |
    bitnumIntr(s1, 17, 3) | bitnumIntr(s0, 17, 2) | bitnumIntr(s1, 25, 1) | bitnumIntr(s0, 25, 0);
  out[4] =
    bitnumIntr(s1, 0, 7) | bitnumIntr(s0, 0, 6) | bitnumIntr(s1, 8, 5) | bitnumIntr(s0, 8, 4) |
    bitnumIntr(s1, 16, 3) | bitnumIntr(s0, 16, 2) | bitnumIntr(s1, 24, 1) | bitnumIntr(s0, 24, 0);
}

/** The DES round function `f(state, subkey)`. */
function feistel(state: number, key: ArrayLike<number>): number {
  // Expansion permutation, split across two words.
  const t1 =
    bitnumIntl(state, 31, 0) | ((state & 0xf0000000) >>> 1) | bitnumIntl(state, 4, 5) |
    bitnumIntl(state, 3, 6) | ((state & 0x0f000000) >>> 3) | bitnumIntl(state, 8, 11) |
    bitnumIntl(state, 7, 12) | ((state & 0x00f00000) >>> 5) | bitnumIntl(state, 12, 17) |
    bitnumIntl(state, 11, 18) | ((state & 0x000f0000) >>> 7) | bitnumIntl(state, 16, 23);
  const t2 =
    bitnumIntl(state, 15, 0) | ((state & 0x0000f000) << 15) | bitnumIntl(state, 20, 5) |
    bitnumIntl(state, 19, 6) | ((state & 0x00000f00) << 13) | bitnumIntl(state, 24, 11) |
    bitnumIntl(state, 23, 12) | ((state & 0x000000f0) << 11) | bitnumIntl(state, 28, 17) |
    bitnumIntl(state, 27, 18) | ((state & 0x0000000f) << 9) | bitnumIntl(state, 0, 23);

  // Key XOR.
  const l0 = ((t1 >>> 24) & 0xff) ^ key[0];
  const l1 = ((t1 >>> 16) & 0xff) ^ key[1];
  const l2 = ((t1 >>> 8) & 0xff) ^ key[2];
  const l3 = ((t2 >>> 24) & 0xff) ^ key[3];
  const l4 = ((t2 >>> 16) & 0xff) ^ key[4];
  const l5 = ((t2 >>> 8) & 0xff) ^ key[5];

  // S-box permutation.
  let out =
    (SBOX1[sboxBit(l0 >>> 2)] << 28) |
    (SBOX2[sboxBit(((l0 & 0x03) << 4) | (l1 >>> 4))] << 24) |
    (SBOX3[sboxBit(((l1 & 0x0f) << 2) | (l2 >>> 6))] << 20) |
    (SBOX4[sboxBit(l2 & 0x3f)] << 16) |
    (SBOX5[sboxBit(l3 >>> 2)] << 12) |
    (SBOX6[sboxBit(((l3 & 0x03) << 4) | (l4 >>> 4))] << 8) |
    (SBOX7[sboxBit(((l4 & 0x0f) << 2) | (l5 >>> 6))] << 4) |
    SBOX8[sboxBit(l5 & 0x3f)];

  // P-box permutation.
  out =
    bitnumIntl(out, 15, 0) | bitnumIntl(out, 6, 1) | bitnumIntl(out, 19, 2) |
    bitnumIntl(out, 20, 3) | bitnumIntl(out, 28, 4) | bitnumIntl(out, 11, 5) |
    bitnumIntl(out, 27, 6) | bitnumIntl(out, 16, 7) | bitnumIntl(out, 0, 8) |
    bitnumIntl(out, 14, 9) | bitnumIntl(out, 22, 10) | bitnumIntl(out, 25, 11) |
    bitnumIntl(out, 4, 12) | bitnumIntl(out, 17, 13) | bitnumIntl(out, 30, 14) |
    bitnumIntl(out, 9, 15) | bitnumIntl(out, 1, 16) | bitnumIntl(out, 7, 17) |
    bitnumIntl(out, 23, 18) | bitnumIntl(out, 13, 19) | bitnumIntl(out, 31, 20) |
    bitnumIntl(out, 26, 21) | bitnumIntl(out, 2, 22) | bitnumIntl(out, 8, 23) |
    bitnumIntl(out, 18, 24) | bitnumIntl(out, 12, 25) | bitnumIntl(out, 29, 26) |
    bitnumIntl(out, 5, 27) | bitnumIntl(out, 21, 28) | bitnumIntl(out, 10, 29) |
    bitnumIntl(out, 3, 30) | bitnumIntl(out, 24, 31);

  return out >>> 0;
}

/**
 * Build the 16 six-byte subkeys. The `& 0xfffffff0` mask (instead of the
 * standard `0x0fffffff`) is what makes this DES incompatible with real DES.
 */
function desKeySetup(key: ArrayLike<number>, decrypt: boolean): Uint8Array {
  let c = 0;
  let d = 0;
  for (let i = 0, j = 31; i < 28; i += 1, j -= 1) c |= bitnum(key, KEY_PERM_C[i], j);
  for (let i = 0, j = 31; i < 28; i += 1, j -= 1) d |= bitnum(key, KEY_PERM_D[i], j);

  const schedule = new Uint8Array(16 * 6);
  for (let i = 0; i < 16; i += 1) {
    const shift = KEY_RND_SHIFT[i];
    c = (((c << shift) | (c >>> (28 - shift))) & 0xfffffff0) >>> 0;
    d = (((d << shift) | (d >>> (28 - shift))) & 0xfffffff0) >>> 0;

    const slot = (decrypt ? 15 - i : i) * 6;
    let j = 0;
    for (; j < 24; j += 1) {
      schedule[slot + (j >> 3)] |= bitnumIntr(c, KEY_COMPRESSION[j], 7 - (j % 8));
    }
    for (; j < 48; j += 1) {
      schedule[slot + (j >> 3)] |= bitnumIntr(d, KEY_COMPRESSION[j] - 27, 7 - (j % 8));
    }
  }
  return schedule;
}

/** Encrypt/decrypt one 8-byte block in place. */
function desCryptBlock(input: Uint8Array, schedule: Uint8Array, offset: number): void {
  let state = initialPermutation(input.subarray(offset, offset + 8));

  for (let idx = 0; idx < 15; idx += 1) {
    const t = state[1];
    state[1] = (feistel(state[1], schedule.subarray(idx * 6, idx * 6 + 6)) ^ state[0]) >>> 0;
    state[0] = t;
  }
  state[0] = (feistel(state[1], schedule.subarray(15 * 6, 15 * 6 + 6)) ^ state[0]) >>> 0;

  const out = new Uint8Array(8);
  inverseInitialPermutation(state, out);
  input.set(out, offset);
}

/** Apply one key pass over every 8-byte block. */
function desPass(bytes: Uint8Array, key: ArrayLike<number>, decrypt: boolean): void {
  const schedule = desKeySetup(key, decrypt);
  for (let offset = 0; offset + 8 <= bytes.length; offset += 8) {
    desCryptBlock(bytes, schedule, offset);
  }
}

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.replace(/\s+/g, "");
  const length = clean.length >> 1;
  const bytes = new Uint8Array(length);
  for (let i = 0; i < length; i += 1) {
    bytes[i] = parseInt(clean.substr(i * 2, 2), 16);
  }
  return bytes;
}

// ---------------------------------------------------------------------------
// DEFLATE / zlib decoder (RFC 1950 + RFC 1951)
// ---------------------------------------------------------------------------

const LENGTH_BASE = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/** Order in which the code-length code lengths are stored in a dynamic block. */
const CODE_LENGTH_ORDER = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/** Least-significant-bit-first bit reader. */
class BitReader {
  private readonly data: Uint8Array;
  private position = 0;

  constructor(data: Uint8Array) {
    this.data = data;
  }

  read(count: number): number {
    let value = 0;
    for (let i = 0; i < count; i += 1) {
      const byte = this.data[this.position >> 3];
      if (byte === undefined) throw new Error("DEFLATE stream truncated");
      value |= ((byte >> (this.position & 7)) & 1) << i;
      this.position += 1;
    }
    return value;
  }

  /** Skip to the next byte boundary (used by stored blocks). */
  align(): void {
    this.position = (this.position + 7) & ~7;
  }

  readByte(): number {
    const byte = this.data[this.position >> 3];
    if (byte === undefined) throw new Error("DEFLATE stream truncated");
    this.position += 8;
    return byte;
  }
}

/** Canonical Huffman table: `counts[len]` entries, symbols sorted by code. */
interface Huffman {
  counts: number[];
  symbols: number[];
  maxBits: number;
}

function buildHuffman(lengths: number[], maxBits: number): Huffman {
  const counts = new Array<number>(maxBits + 1).fill(0);
  for (const length of lengths) counts[length] += 1;
  counts[0] = 0;

  const offsets = new Array<number>(maxBits + 2).fill(0);
  for (let bits = 1; bits <= maxBits; bits += 1) offsets[bits + 1] = offsets[bits] + counts[bits];

  const symbols = new Array<number>(lengths.length).fill(0);
  for (let symbol = 0; symbol < lengths.length; symbol += 1) {
    if (lengths[symbol] > 0) symbols[offsets[lengths[symbol]]++] = symbol;
  }
  return { counts, symbols, maxBits };
}

function decodeSymbol(reader: BitReader, huffman: Huffman): number {
  let code = 0;
  let first = 0;
  let index = 0;
  for (let bits = 1; bits <= huffman.maxBits; bits += 1) {
    code |= reader.read(1);
    const count = huffman.counts[bits];
    if (code - first < count) return huffman.symbols[index + (code - first)];
    index += count;
    first = (first + count) << 1;
    code <<= 1;
  }
  throw new Error("invalid Huffman code");
}

function fixedTables(): { literal: Huffman; distance: Huffman } {
  const literalLengths = new Array<number>(288).fill(0);
  for (let i = 0; i < 144; i += 1) literalLengths[i] = 8;
  for (let i = 144; i < 256; i += 1) literalLengths[i] = 9;
  for (let i = 256; i < 280; i += 1) literalLengths[i] = 7;
  for (let i = 280; i < 288; i += 1) literalLengths[i] = 8;
  return {
    literal: buildHuffman(literalLengths, 15),
    distance: buildHuffman(new Array<number>(30).fill(5), 15),
  };
}

function dynamicTables(reader: BitReader): { literal: Huffman; distance: Huffman } {
  const literalCount = reader.read(5) + 257;
  const distanceCount = reader.read(5) + 1;
  const codeLengthCount = reader.read(4) + 4;

  const codeLengthLengths = new Array<number>(19).fill(0);
  for (let i = 0; i < codeLengthCount; i += 1) {
    codeLengthLengths[CODE_LENGTH_ORDER[i]] = reader.read(3);
  }
  const codeLengthHuffman = buildHuffman(codeLengthLengths, 7);

  const lengths = new Array<number>(literalCount + distanceCount).fill(0);
  for (let i = 0; i < lengths.length; ) {
    const symbol = decodeSymbol(reader, codeLengthHuffman);
    if (symbol < 16) {
      lengths[i++] = symbol;
    } else if (symbol === 16) {
      const previous = lengths[i - 1];
      for (let repeat = reader.read(2) + 3; repeat > 0 && i < lengths.length; repeat -= 1) {
        lengths[i++] = previous;
      }
    } else if (symbol === 17) {
      i += reader.read(3) + 3;
    } else {
      i += reader.read(7) + 11;
    }
  }

  return {
    literal: buildHuffman(lengths.slice(0, literalCount), 15),
    distance: buildHuffman(lengths.slice(literalCount), 15),
  };
}

/** Growable output window. */
class Output {
  private buffer = new Uint8Array(1 << 14);
  private length = 0;

  private reserve(extra: number): void {
    if (this.length + extra <= this.buffer.length) return;
    let capacity = this.buffer.length;
    while (capacity < this.length + extra) capacity *= 2;
    const next = new Uint8Array(capacity);
    next.set(this.buffer.subarray(0, this.length));
    this.buffer = next;
  }

  pushByte(byte: number): void {
    this.reserve(1);
    this.buffer[this.length++] = byte;
  }

  /** Copy `count` bytes starting `distance` back (may overlap). */
  copyBack(distance: number, count: number): void {
    if (distance > this.length) throw new Error("invalid DEFLATE back-reference");
    this.reserve(count);
    for (let i = 0; i < count; i += 1) {
      this.buffer[this.length] = this.buffer[this.length - distance];
      this.length += 1;
    }
  }

  finish(): Uint8Array {
    return this.buffer.slice(0, this.length);
  }
}

/** Decode one huffman-coded block body (literals + length/distance pairs). */
function inflateBlockBody(reader: BitReader, literal: Huffman, distance: Huffman, output: Output): void {
  for (;;) {
    const symbol = decodeSymbol(reader, literal);
    if (symbol < 256) {
      output.pushByte(symbol);
      continue;
    }
    if (symbol === 256) return;

    const lengthIndex = symbol - 257;
    const length = LENGTH_BASE[lengthIndex] + reader.read(LENGTH_EXTRA[lengthIndex]);
    const distanceSymbol = decodeSymbol(reader, distance);
    const copyDistance = DIST_BASE[distanceSymbol] + reader.read(DIST_EXTRA[distanceSymbol]);
    output.copyBack(copyDistance, length);
  }
}

/** Raw DEFLATE stream (no wrapper) → decompressed bytes. */
function inflateRaw(data: Uint8Array): Uint8Array {
  const reader = new BitReader(data);
  const output = new Output();

  for (;;) {
    const isFinal = reader.read(1) === 1;
    const type = reader.read(2);
    if (type === 0) {
      reader.align();
      const length = reader.readByte() | (reader.readByte() << 8);
      reader.readByte();
      reader.readByte();
      for (let i = 0; i < length; i += 1) output.pushByte(reader.readByte());
    } else if (type === 1) {
      const tables = fixedTables();
      inflateBlockBody(reader, tables.literal, tables.distance, output);
    } else if (type === 2) {
      const tables = dynamicTables(reader);
      inflateBlockBody(reader, tables.literal, tables.distance, output);
    } else {
      throw new Error("invalid DEFLATE block type");
    }
    if (isFinal) break;
  }

  return output.finish();
}

/** zlib (RFC 1950) → decompressed bytes; falls back to raw DEFLATE. */
function inflate(data: Uint8Array): Uint8Array {
  const hasZlibHeader =
    data.length >= 2 && (data[0] & 0x0f) === 8 && ((data[0] << 8) | data[1]) % 31 === 0;
  return inflateRaw(hasZlibHeader ? data.subarray(2) : data);
}

/**
 * Decrypt a QQ QRC hex payload into word-by-word lyric text.
 *
 * @throws when the payload is not valid QRC (bad hex, failed DES or inflate).
 */
export async function decryptQrc(hex: string): Promise<string> {
  const bytes = hexToBytes(hex);
  if (bytes.length === 0 || bytes.length % 8 !== 0) {
    throw new Error("QRC payload is not block aligned");
  }

  desPass(bytes, KEY1, true);
  desPass(bytes, KEY2, false);
  desPass(bytes, KEY3, true);

  return new TextDecoder("utf-8", { fatal: false }).decode(inflate(bytes));
}
