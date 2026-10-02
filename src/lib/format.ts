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

/** Human-readable byte size (binary units). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const value = bytes / Math.pow(1024, exponent);
  return `${exponent === 0 || value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[exponent]}`;
}

/** Human-readable transfer rate, e.g. "1.4 MB/s". */
export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}
