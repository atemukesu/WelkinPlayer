//! Lyric fetching: NetEase, QQ Music and the AMLL TTML database.
//!
//! Provider logic lives here; the actual HTTP request is tunnelled through
//! `lyric_http_get` because the music platforms do not allow cross-origin
//! requests from the webview. Platform searches are memoized through
//! [`LyricResolver`] so a single title/artist lookup serves every provider.

import { invoke } from "../api";
import { decryptQrc } from "./qrc";
import { mergeQrcTranslation } from "./qrcMerge";
import { lyricLog } from "./lyricLog";

/** Title/artist/album used to look a track up on the music platforms. */
export interface LyricQuery {
  title: string;
  artist: string;
  album?: string;
}

/** A song hit from one platform's search. */
export interface LyricMatch {
  /** Primary platform id: the NetEase song id or the QQ song mid. */
  id: string;
  /** Secondary platform id (QQ exposes both a `mid` and a numeric `id`). */
  altId?: string;
  title: string;
  artist: string;
  album: string;
}

/** Platform identifiers resolved from a title/artist lookup. */
export interface LyricIds {
  netease?: string;
  /** QQ song mid. */
  qqMid?: string;
  /** QQ numeric song id — required for word-by-word QRC. */
  qqId?: string;
}

/** A fetched lyric plus the platform id it was resolved with. */
export interface LyricFetchResult {
  content: string;
  /** Source id for provenance tagging, e.g. `ncm/186016` or `qq/97773`. */
  sourceId?: string;
}

const NETEASE_REFERER = "https://music.163.com/";
const QQ_REFERER = "https://y.qq.com/";
const AMLL_BASE = "https://api.amll.dev";

/** Raw text GET through the Rust tunnel (bypasses CORS). */
async function tunnel(url: string, referer: string): Promise<string> {
  lyricLog("info", `-> ${url}`);
  try {
    const body = await invoke<string>("lyric_http_get", { url, referer });
    lyricLog("info", `<- ${url}`, `${body.length} chars`);
    return body;
  } catch (error) {
    lyricLog("error", `x- ${url}`, error);
    throw error;
  }
}

/** Decode the HTML entities QQ/NetEase embed in lyric payloads. */
function decodeEntities(input: string): string {
  return input
    .replace(/&apos;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&nbsp;/g, " ")
    .replace(/&#(\d+);/g, (_match, dec: string) => String.fromCharCode(Number(dec)))
    .replace(/&#x([0-9a-fA-F]+);/g, (_match, hex: string) => String.fromCharCode(parseInt(hex, 16)))
    .replace(/&amp;/g, "&");
}

/** Join a main lyric with an optional translation track, keeping time tags. */
function mergeLyrics(main: string | null | undefined, translation: string | null | undefined): string | null {
  const primary = decodeEntities(main ?? "").trim();
  if (!primary) return null;
  const secondary = decodeEntities(translation ?? "").trim();
  return secondary ? `${primary}\n${secondary}` : primary;
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/** Lowercase, drop bracketed qualifiers and punctuation for comparison. */
function normalizeText(value: string): string {
  return value
    .toLowerCase()
    .replace(/[[(（【][^)\]}）】]*[)\]}）】]/g, " ")
    .replace(/\b(feat|ft|with|remaster(?:ed)?|version|ver)\b\.?/g, " ")
    .replace(/[^\p{L}\p{N}]+/gu, "");
}

/** Alternate recordings that should lose to the original studio version. */
const VERSION_MARKERS = /(live|concert|remix|cover|karaoke|instrumental|acoustic|demo|伴奏|翻唱|现场|纯音乐|dj)/i;

/** Higher is a better match; 0 means unrelated. */
function matchScore(query: LyricQuery, title: string, artist: string, album: string): number {
  const wantTitle = normalizeText(query.title);
  const gotTitle = normalizeText(title);
  const wantArtist = normalizeText(query.artist);
  const gotArtist = normalizeText(artist);

  let score = 0;
  if (wantTitle && gotTitle) {
    if (wantTitle === gotTitle) score += 6;
    else if (gotTitle.includes(wantTitle) || wantTitle.includes(gotTitle)) score += 3;
  }
  if (wantArtist && gotArtist) {
    const artists = gotArtist.split(/[,&、/]/).map((name) => name.trim());
    if (gotArtist === wantArtist || artists.includes(wantArtist)) score += 4;
    else if (gotArtist.includes(wantArtist) || wantArtist.includes(gotArtist)) score += 2;
  }
  const wantAlbum = normalizeText(query.album ?? "");
  if (wantAlbum && wantAlbum === normalizeText(album)) score += 1;

  if (score > 0) {
    if (title.trim().toLowerCase() === query.title.trim().toLowerCase()) score += 2;
    if (VERSION_MARKERS.test(title)) score -= 2;
  }
  return score;
}

interface Comparable {
  title: string;
  artist: string;
  album: string;
}

/** Pick the best-scoring item, ignoring unrelated results. */
function pickBest<T>(query: LyricQuery, items: T[], toComparable: (item: T) => Comparable): T | null {
  let best: T | null = null;
  let bestScore = 0;
  for (const item of items) {
    const comparable = toComparable(item);
    const score = matchScore(query, comparable.title, comparable.artist, comparable.album);
    if (score > bestScore) {
      best = item;
      bestScore = score;
    }
  }
  return bestScore > 0 ? best : null;
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

interface NeteaseSong {
  id?: number;
  name?: string;
  artists?: Array<{ name?: string }>;
  album?: { name?: string };
}

/** Search NetEase and return the best matching song id. */
export async function searchNetease(query: LyricQuery): Promise<LyricMatch | null> {
  const text = `${query.title} ${query.artist}`.trim();
  if (!text) return null;
  const url =
    "https://music.163.com/api/search/get/web?type=1&offset=0&total=true&limit=10&s=" +
    encodeURIComponent(text);
  const body = await tunnel(url, NETEASE_REFERER);
  const parsed = JSON.parse(body) as { result?: { songs?: NeteaseSong[] } };
  const songs = parsed.result?.songs ?? [];
  const matches: LyricMatch[] = songs
    .filter((song): song is NeteaseSong & { id: number } => typeof song.id === "number")
    .map((song) => ({
      id: String(song.id),
      title: song.name ?? "",
      artist: (song.artists ?? []).map((artist) => artist.name ?? "").join(", "),
      album: song.album?.name ?? "",
    }));
  const best = pickBest(query, matches, (match) => match);
  lyricLog(
    best ? "info" : "warn",
    `netease search "${text}"`,
    best
      ? `${matches.length} results, picked ${best.id} ${best.title} / ${best.artist}`
      : `${matches.length} results, no confident match`,
  );
  return best;
}

interface QqSong {
  songid?: number;
  songmid?: string;
  songname?: string;
  singer?: Array<{ name?: string }>;
  albumname?: string;
}

/**
 * Search QQ Music.
 *
 * Uses the `search_for_qq_cp` endpoint: the newer
 * `musicu.fcg`/`DoSearchForQQMusicDesktop` route started answering
 * `code 2001` with an empty song list (it wants a signed request), whereas this
 * one still returns plain results. Note the field names differ from musicu —
 * `songmid`/`songid`/`songname`/`albumname` instead of `mid`/`id`/`title`/`album`.
 */
export async function searchQq(query: LyricQuery): Promise<LyricMatch | null> {
  const text = `${query.title} ${query.artist}`.trim();
  if (!text) return null;
  const url = `https://c.y.qq.com/soso/fcgi-bin/search_for_qq_cp?format=json&w=${encodeURIComponent(text)}&n=10&p=1`;
  const body = await tunnel(url, QQ_REFERER);
  const parsed = JSON.parse(body) as {
    data?: { song?: { list?: QqSong[] } };
  };
  const songs = parsed.data?.song?.list ?? [];
  const matches: LyricMatch[] = songs
    .filter(
      (song): song is QqSong & { songmid: string } =>
        typeof song.songmid === "string" && song.songmid.length > 0,
    )
    .map((song) => ({
      id: song.songmid,
      altId: typeof song.songid === "number" ? String(song.songid) : undefined,
      title: song.songname ?? "",
      artist: (song.singer ?? []).map((singer) => singer.name ?? "").join(", "),
      album: song.albumname ?? "",
    }));
  const best = pickBest(query, matches, (match) => match);
  lyricLog(
    best ? "info" : "warn",
    `qq search "${text}"`,
    best
      ? `${matches.length} results, picked mid=${best.id} id=${best.altId ?? "-"} ${best.title} / ${best.artist}`
      : `${matches.length} results, no confident match`,
  );
  return best;
}

// ---------------------------------------------------------------------------
// Lyrics
// ---------------------------------------------------------------------------

/** NetEase line-level LRC plus the optional translation track. */
export async function lyricFromNetease(id: string): Promise<string | null> {
  const url = `https://music.163.com/api/song/lyric?id=${encodeURIComponent(id)}&lv=-1&kv=-1&tv=-1`;
  const body = await tunnel(url, NETEASE_REFERER);
  const parsed = JSON.parse(body) as {
    lrc?: { lyric?: string };
    tlyric?: { lyric?: string };
  };
  const merged = mergeLyrics(parsed.lrc?.lyric, parsed.tlyric?.lyric);
  lyricLog(
    merged ? "info" : "warn",
    `netease lyric ${id}`,
    merged
      ? `${merged.length} chars${parsed.tlyric?.lyric ? " + translation" : ""}`
      : "empty",
  );
  return merged;
}
/** QQ Music line-level LRC plus the optional translation track. */
export async function lyricFromQq(mid: string): Promise<string | null> {
  const url =
    "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid=" +
    encodeURIComponent(mid) +
    "&format=json&nobase64=1&g_tk=5381";
  const body = await tunnel(url, QQ_REFERER);
  const parsed = JSON.parse(body) as { lyric?: string; trans?: string };
  const merged = mergeLyrics(parsed.lyric, parsed.trans);
  lyricLog(
    merged ? "info" : "warn",
    `qq lyric (line-level) ${mid}`,
    merged
      ? `${merged.length} chars${parsed.trans ? " + translation" : ""}`
      : "empty",
  );
  return merged;
}

/** Read one CDATA child of the `<lyric>` element returned by `lyric_download.fcg`. */
function cdata(body: string, tag: string): string {
  const match = new RegExp(`<${tag}\\b[^>]*><!\\[CDATA\\[([\\s\\S]*?)\\]\\]></${tag}>`).exec(body);
  return match?.[1]?.trim() ?? "";
}

/**
 * QQ word-by-word lyrics (QRC).
 *
 * `lrctype=4` returns the QRC payload hex-encrypted with QQ's custom DES and
 * zlib-compressed; the plaintext is a `<QrcInfos>` XML document whose
 * `LyricContent` attribute holds the actual word-timed lyric.
 */
export async function lyricFromQqQrc(songId: string): Promise<string | null> {
  const url =
    "https://c.y.qq.com/qqmusic/fcgi-bin/lyric_download.fcg?version=15&miniversion=82&lrctype=4&musicid=" +
    encodeURIComponent(songId);
  const body = await tunnel(url, "https://y.qq.com/portal/player.html");

  const encrypted = cdata(body, "content");
  if (!encrypted) {
    lyricLog("warn", `qq qrc ${songId}: response has no <content> payload`);
    return null;
  }

  lyricLog("info", `qq qrc ${songId}: decrypting ${encrypted.length} hex chars`);
  const xml = await decryptQrc(encrypted);
  const start = xml.indexOf('LyricContent="');
  const end = xml.lastIndexOf('"');
  if (start < 0 || end <= start) {
    lyricLog("warn", `qq qrc ${songId}: decrypted but LyricContent is missing`);
    return null;
  }

  const qrc = xml
    .slice(start + 'LyricContent="'.length, end)
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, "&")
    .trim();
  if (!qrc) {
    lyricLog("warn", `qq qrc (word-by-word) ${songId}`, "empty");
    return null;
  }

  // <contentts> is the translation track and arrives as plaintext LRC, so it
  // must not be decrypted. Merging it into the QRC keeps word timings (TTML).
  const translation = cdata(body, "contentts");
  const content = translation ? mergeQrcTranslation(qrc, translation) : qrc;
  lyricLog(
    "info",
    `qq qrc (word-by-word) ${songId}`,
    `${content.length} chars${translation ? " + translation" : ""}`,
  );
  return content;
}

/**
 * QQ lyrics with the best format available: word-by-word QRC first, then the
 * line-level LRC (which carries the translation track).
 */
export async function lyricFromQqBest(ids: LyricIds): Promise<LyricFetchResult | null> {
  if (ids.qqId) {
    try {
      const qrc = await lyricFromQqQrc(ids.qqId);
      if (qrc) return { content: qrc, sourceId: `qq/${ids.qqId}` };
      lyricLog("warn", `qq ${ids.qqId}: QRC empty, falling back to line-level LRC`);
    } catch (error) {
      lyricLog("warn", `qq ${ids.qqId}: QRC failed, falling back to line-level LRC`, error);
    }
  } else {
    lyricLog("warn", "qq: numeric song id missing, cannot try word-by-word QRC");
  }

  if (ids.qqMid) {
    const lrc = await lyricFromQq(ids.qqMid).catch((error) => {
      lyricLog("error", `qq lyric (line-level) ${ids.qqMid} failed`, error);
      return null;
    });
    if (lrc) return { content: lrc, sourceId: `qq/${ids.qqMid}` };
  }

  lyricLog(
    "warn",
    "qq: no lyrics found",
    `mid=${ids.qqMid ?? "-"} id=${ids.qqId ?? "-"}`,
  );
  return null;
}

interface AmllItem {
  filename?: string;
  musicNames?: string[];
  artistNames?: string[];
  albumNames?: string[];
}

/** Fetch one AMLL entry by an explicit query parameter. */
async function amllGet(param: string): Promise<string | null> {
  try {
    const body = await tunnel(`${AMLL_BASE}/v1/lyrics/get?${param}`, AMLL_BASE);
    const parsed = JSON.parse(body) as { data?: { lyrics?: string } };
    const lyrics = parsed.data?.lyrics;
    const hit = lyrics && lyrics.trim() ? lyrics : null;
    lyricLog(
      hit ? "info" : "warn",
      `amll get ${param}`,
      hit ? `${hit.length} chars` : "no entry",
    );
    return hit;
  } catch (error) {
    lyricLog("warn", `amll get ${param} failed`, error);
    return null;
  }
}

/**
 * Last-resort AMLL lookup: fuzzy search by title/artist, then fetch the best
 * entry by its exact filename. Platform-id lookups are preferred because the
 * database's QQ ids are stored inconsistently (mid for some entries, the
 * numeric song id for others).
 */
async function amllSearchByName(query: LyricQuery): Promise<string | null> {
  if (!query.title.trim()) return null;
  try {
    const params = new URLSearchParams();
    params.set("musicName", query.title);
    if (query.artist.trim()) params.set("artistName", query.artist);
    if (query.album?.trim()) params.set("albumName", query.album);
    params.set("pageSize", "5");
    const body = await tunnel(`${AMLL_BASE}/v1/lyrics/search?${params}`, AMLL_BASE);
    const parsed = JSON.parse(body) as { data?: { items?: AmllItem[] } };
    const items = parsed.data?.items ?? [];
    const best = pickBest(query, items, (item) => ({
      title: item.musicNames?.[0] ?? "",
      artist: (item.artistNames ?? []).join(", "),
      album: item.albumNames?.[0] ?? "",
    }));
    if (!best?.filename) {
      lyricLog(
        "warn",
        "amll name search: no entry",
        `"${query.title} ${query.artist}" (${items.length} results)`,
      );
      return null;
    }
    lyricLog("info", "amll name search matched", `${best.filename} (${items.length} results)`);
    return amllGet(`filename=${encodeURIComponent(best.filename)}`);
  } catch (error) {
    lyricLog("warn", "amll name search failed", error);
    return null;
  }
}

/** Word-by-word TTML from the AMLL database, looked up by platform id. */
export async function lyricFromAmll(
  ids: LyricIds,
  query: LyricQuery,
): Promise<LyricFetchResult | null> {
  // Some database entries store the QQ mid, others the numeric song id.
  const candidates: Array<{ param: string; sourceId: string }> = [];
  if (ids.netease) {
    candidates.push({ param: `ncmMusicId=${encodeURIComponent(ids.netease)}`, sourceId: `ncm/${ids.netease}` });
  }
  if (ids.qqMid) {
    candidates.push({ param: `qqMusicId=${encodeURIComponent(ids.qqMid)}`, sourceId: `qq/${ids.qqMid}` });
  }
  if (ids.qqId) {
    candidates.push({ param: `qqMusicId=${encodeURIComponent(ids.qqId)}`, sourceId: `qq/${ids.qqId}` });
  }
  lyricLog(
    "info",
    "amll: trying platform ids",
    candidates.length ? candidates.map((candidate) => candidate.sourceId).join(", ") : "none resolved",
  );
  for (const candidate of candidates) {
    const lyrics = await amllGet(candidate.param);
    if (lyrics) return { content: lyrics, sourceId: candidate.sourceId };
  }
  lyricLog("warn", "amll: no platform id hit; falling back to name search");
  const fallback = await amllSearchByName(query);
  return fallback ? { content: fallback } : null;
}

/**
 * Lazily resolve (and memoize) the platform ids for one track.
 *
 * A single title/artist search per platform is shared by every provider, so
 * AMLL does not need to rely on fuzzy search: it is looked up by the ids that
 * NetEase/QQ already resolved.
 */
export class LyricResolver {
  private readonly query: LyricQuery;
  private readonly ids: LyricIds = {};
  private neteaseResolved = false;
  private qqResolved = false;

  constructor(query: LyricQuery) {
    this.query = query;
  }

  /** Known ids without triggering a lookup. */
  get cached(): LyricIds {
    return this.ids;
  }

  /** Resolve and memoize the NetEase id. */
  async netease(): Promise<LyricIds> {
    if (!this.neteaseResolved) {
      this.neteaseResolved = true;
      this.ids.netease = (await searchNetease(this.query))?.id;
      lyricLog("info", "resolved netease id", this.ids.netease ?? "none");
    }
    return this.ids;
  }

  /** Resolve and memoize both QQ identifiers (mid + numeric id). */
  async qq(): Promise<LyricIds> {
    if (!this.qqResolved) {
      this.qqResolved = true;
      const match = await searchQq(this.query);
      this.ids.qqMid = match?.id;
      this.ids.qqId = match?.altId;
      lyricLog(
        "info",
        "resolved qq ids",
        `mid=${this.ids.qqMid ?? "-"} id=${this.ids.qqId ?? "-"}`,
      );
    }
    return this.ids;
  }
}
