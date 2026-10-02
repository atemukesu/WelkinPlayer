/**
 * Copyright 2026 Atemukesu
 * SPDX-License-Identifier: GPL-3.0-only
 */

import type { Component } from "vue";
import { Disc3, HardDrive, Heart, Library, ListMusic, Mic2, Settings2 } from "@lucide/vue";

export type View = "library" | "tracks" | "favorites" | "stats" | "artists" | "albums" | "sources" | "source-config" | "playlists" | "playlist-new" | "lyrics-edit" | "track-edit" | "track-info" | "settings" | "sponsor" | "player";
export type Theme = "light" | "dark";
export type Accent = "amber" | "orange" | "cyan" | "red" | "green" | "blue";

export interface NavItem {
  id: View;
  labelKey: string;
  index: string;
  icon: Component;
}

export const navItems: NavItem[] = [
  { id: "library", labelKey: "nav.library", index: "01", icon: Library },
];

/** Settings entry, pinned to the bottom of the navigation. */
export const settingsNavItem: NavItem = { id: "settings", labelKey: "nav.settings", index: "02", icon: Settings2 };

/** Collections shown in a dedicated sidebar group, below the primary nav. */
export const collectionNavItems: NavItem[] = [
  { id: "favorites", labelKey: "nav.favorites", index: "F", icon: Heart },
  { id: "tracks", labelKey: "nav.tracks", index: "A", icon: ListMusic },
];

/** Library classifications, grouped with the collections in the navigation. */
export const classificationNavItems: NavItem[] = [
  { id: "artists", labelKey: "nav.artists", index: "G", icon: Mic2 },
  { id: "albums", labelKey: "nav.albums", index: "B", icon: Disc3 },
  { id: "sources", labelKey: "nav.sources", index: "S", icon: HardDrive },
];

export const accents: { id: Accent; key: string; swatch: string }[] = [
  { id: "amber", key: "accent.amber", swatch: "#f0a500" },
  { id: "orange", key: "accent.orange", swatch: "#ff6a00" },
  { id: "cyan", key: "accent.cyan", swatch: "#00b8d4" },
  { id: "red", key: "accent.red", swatch: "#e63946" },
  { id: "green", key: "accent.green", swatch: "#16c79a" },
  { id: "blue", key: "accent.blue", swatch: "#3d7dff" },
];
