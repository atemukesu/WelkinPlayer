export function initial(track: { title: string }) {
  return track.title.charAt(0);
}

/**
 * Whether a track's cover is still being resolved. While true (and no cover is
 * known yet) the UI shows a loading spinner; once resolution settles without a
 * cover it falls back to the colour block + initial.
 */
export function coverPending(track: { cover?: string; metaLoaded?: boolean; metaFailed?: boolean } | null | undefined): boolean {
  if (!track) return false;
  return !track.cover && !track.metaLoaded && !track.metaFailed;
}

export function percent(value: number) {
  return `${value}%`;
}

export function pad(value: number) {
  return String(value).padStart(2, "0");
}
