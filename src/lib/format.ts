export function initial(track: { title: string }) {
  return track.title.charAt(0);
}

export function percent(value: number) {
  return `${value}%`;
}

export function pad(value: number) {
  return String(value).padStart(2, "0");
}
