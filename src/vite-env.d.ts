/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

/// <reference types="vite/client" />

interface Window {
  /** Timestamp (ms) set by the inline script in index.html when the splash first painted. */
  __welkinSplashStart?: number;
}

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<{}, {}, any>;
  export default component;
}
