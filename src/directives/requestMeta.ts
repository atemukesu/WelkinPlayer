/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import type { Directive } from "vue";
import type { Track } from "../stores/player";
import { useMetadataStore } from "../stores/metadata";

interface MetaElement extends HTMLElement {
  __metaObserver?: IntersectionObserver;
  __metaTimer?: number;
}

/** Small delay so fast scrolling past a row never enqueues it. */
const VISIBLE_DELAY_MS = 150;
/** Start fetching slightly before a row scrolls into view. */
const ROOT_MARGIN = "300px";

function teardown(el: MetaElement) {
  el.__metaObserver?.disconnect();
  el.__metaObserver = undefined;
  if (el.__metaTimer) {
    window.clearTimeout(el.__metaTimer);
    el.__metaTimer = 0;
  }
}

function observe(el: MetaElement, track: Track | undefined) {
  if (!track || track.metaLoaded || !track.path || !track.sourceId) return;
  const store = useMetadataStore();
  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      observer.disconnect();
      el.__metaObserver = undefined;
      el.__metaTimer = window.setTimeout(() => {
        el.__metaTimer = 0;
        store.request(track);
      }, VISIBLE_DELAY_MS);
    }
  }, { rootMargin: ROOT_MARGIN });
  el.__metaObserver = observer;
  observer.observe(el);
}

/**
 * `v-request-meta="track"` enqueues a track's metadata once its element is
 * (nearly) visible. Rows are virtualized, so mounting already approximates
 * "on screen"; the observer handles the rest and the delay drops fast scrolls.
 */
export const requestMeta: Directive<MetaElement, Track | undefined> = {
  mounted(el, binding) {
    observe(el, binding.value);
  },
  updated(el, binding) {
    if (binding.value === binding.oldValue) return;
    teardown(el);
    observe(el, binding.value);
  },
  unmounted(el) {
    teardown(el);
  },
};
