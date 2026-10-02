import { ref, watch } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";
import { isDesktop } from "../lib/desktopLyric";

/** Local-only preference: how the main window's close button behaves. */
const STORAGE_KEY = "welkin-close-to-tray";

/**
 * Window closing behaviour. Kept client-local (like theme/accent) because it is
 * a per-device preference, not part of the synced profile. The backend owns the
 * actual close handler, so the current value is pushed to it on boot and on
 * every change; Android has no tray and hides the setting entirely.
 */
export const useWindowBehaviorStore = defineStore("windowBehavior", () => {
  const closeToTray = ref(isDesktop() && localStorage.getItem(STORAGE_KEY) === "1");

  /** Send the current preference to the backend close handler. */
  function sync() {
    if (!isDesktop()) return;
    void invoke("set_close_to_tray", { closeToTray: closeToTray.value }).catch(() => {});
  }

  watch(closeToTray, (value) => {
    if (!isDesktop()) return;
    localStorage.setItem(STORAGE_KEY, value ? "1" : "0");
    sync();
  });

  // Push the persisted value once so the Rust close handler matches the UI from
  // the very first close of this session.
  sync();

  return { closeToTray, sync };
});
