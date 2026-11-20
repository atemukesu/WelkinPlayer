/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/**
 * Shared-element handover for the smart transition. The incoming title is read
 * from its inline spot beside the artist before the track swaps, then a clone
 * flies to the real title position with a touch of motion blur. Width and font
 * size are animated rather than scaled so the text never stretches; the target
 * title fades in as the clone lands.
 */

export interface TitleFlightRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface TitleFlightSnapshot {
  text: string;
  rect: TitleFlightRect;
  color: string;
  fontFamily: string;
  fontSize: string;
  fontWeight: string;
  letterSpacing: string;
  lineHeight: string;
}

export interface TitleFlightOptions {
  duration?: number;
}

/** First element that is actually laid out (the other responsive variant is hidden). */
export function firstVisibleElement(...elements: Array<HTMLElement | null | undefined>): HTMLElement | null {
  for (const element of elements) {
    if (!element) continue;
    const rect = element.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) return element;
  }
  return null;
}

/** Snapshot an inline title before Vue re-renders the track that owns it. */
export function captureTitleFlight(element: HTMLElement | null | undefined): TitleFlightSnapshot | null {
  if (!element) return null;
  const rect = element.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  const text = (element.textContent ?? "").trim();
  if (!text) return null;
  const style = getComputedStyle(element);
  return {
    text,
    rect: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
    color: style.color,
    fontFamily: style.fontFamily,
    fontSize: style.fontSize,
    fontWeight: style.fontWeight,
    letterSpacing: style.letterSpacing,
    lineHeight: style.lineHeight,
  };
}

/** Animations still owning a title element, so a second handover can cancel them. */
const targetAnimations = new WeakMap<HTMLElement, Animation[]>();

function clearTarget(target: HTMLElement) {
  const animations = targetAnimations.get(target);
  if (animations) {
    for (const animation of animations) animation.cancel();
    targetAnimations.delete(target);
  }
  target.style.opacity = "";
}

/** Fly a captured inline title into the freshly rendered title element. */
export function animateTitleFlight(snapshot: TitleFlightSnapshot, target: HTMLElement | null | undefined, options: TitleFlightOptions = {}) {
  if (!target) return;
  if (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
  const targetRect = target.getBoundingClientRect();
  if (targetRect.width <= 0 || targetRect.height <= 0) return;

  const targetStyle = getComputedStyle(target);
  const duration = options.duration ?? 340;
  const dx = targetRect.left - snapshot.rect.left;
  const dy = targetRect.top - snapshot.rect.top;

  const clone = document.createElement("span");
  clone.textContent = snapshot.text;
  clone.setAttribute("aria-hidden", "true");
  Object.assign(clone.style, {
    position: "fixed",
    left: `${snapshot.rect.left}px`,
    top: `${snapshot.rect.top}px`,
    width: `${snapshot.rect.width}px`,
    margin: "0",
    padding: "0",
    overflow: "hidden",
    whiteSpace: "nowrap",
    textOverflow: "clip",
    pointerEvents: "none",
    zIndex: "100",
    transformOrigin: "left top",
    willChange: "transform, width, font-size, filter, color",
    color: snapshot.color,
    fontFamily: snapshot.fontFamily,
    fontSize: snapshot.fontSize,
    fontWeight: snapshot.fontWeight,
    letterSpacing: snapshot.letterSpacing,
    lineHeight: snapshot.lineHeight,
  });
  document.body.appendChild(clone);

  clearTarget(target);
  target.style.opacity = "0";

  // Mid-flight width/font-size keep the text readable as it grows towards the
  // title size; the brief blur sells the motion without becoming a flashy zoom.
  const widthMid = snapshot.rect.width + (targetRect.width - snapshot.rect.width) * 0.55;
  const flight = clone.animate(
    [
      {
        transform: "translate3d(0, 0, 0)",
        width: `${snapshot.rect.width}px`,
        fontSize: snapshot.fontSize,
        lineHeight: snapshot.lineHeight,
        color: snapshot.color,
        filter: "blur(0px)",
        opacity: 1,
      },
      {
        offset: 0.55,
        transform: `translate3d(${dx * 0.55}px, ${dy * 0.55}px, 0)`,
        width: `${widthMid}px`,
        fontSize: `${(parseFloat(snapshot.fontSize) + (parseFloat(targetStyle.fontSize) - parseFloat(snapshot.fontSize)) * 0.55) || 0}px`,
        lineHeight: targetStyle.lineHeight,
        color: targetStyle.color,
        filter: "blur(1.2px)",
        opacity: 1,
      },
      {
        transform: `translate3d(${dx}px, ${dy}px, 0)`,
        width: `${targetRect.width}px`,
        fontSize: targetStyle.fontSize,
        lineHeight: targetStyle.lineHeight,
        color: targetStyle.color,
        filter: "blur(0px)",
        opacity: 1,
      },
    ],
    { duration, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)", fill: "both" },
  );

  const reveal = target.animate(
    [
      { opacity: 0, offset: 0 },
      { opacity: 0, offset: 0.7 },
      { opacity: 1, offset: 1 },
    ],
    { duration, easing: "ease-out", fill: "both" },
  );
  const animations = [flight, reveal];
  targetAnimations.set(target, animations);

  const finish = () => {
    clone.remove();
    // A newer handover may already own the target; only the owner may restore it.
    if (targetAnimations.get(target) !== animations) return;
    targetAnimations.delete(target);
    flight.cancel();
    reveal.cancel();
    target.style.opacity = "";
  };
  flight.finished.then(finish, finish);
}
