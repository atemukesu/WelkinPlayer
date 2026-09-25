import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { load, type Store } from "@tauri-apps/plugin-store";
import { describeError, invoke } from "../api";

export type WebdavStatus = "idle" | "connecting" | "connected" | "error";

/** Mirrors the `KeychainStatus` struct returned by the Rust commands. */
interface KeychainStatus {
  available: boolean;
  password: string | null;
  warning: string | null;
}

const SETTINGS_FILE = "settings.json";
const URL_KEY = "webdav.url";
const USERNAME_KEY = "webdav.username";

export const useWebdavStore = defineStore("webdav", () => {
  const url = ref("");
  const username = ref("");
  /**
   * Password held in memory only. It is written to / removed from the OS
   * keychain by [`persist`], never to `settings.json`.
   */
  const password = ref("");
  const status = ref<WebdavStatus>("idle");
  const error = ref("");
  const connectedAt = ref<number | null>(null);
  const hydrated = ref(false);
  const keychainAvailable = ref(true);
  const saving = ref(false);

  /** Username last written to disk, so the keychain entry can be re-indexed. */
  let persistedUsername = "";
  let store: Store | null = null;

  const isConnected = computed(() => status.value === "connected");
  const isBusy = computed(() => status.value === "connecting");

  async function settings() {
    if (!store) {
      store = await load(SETTINGS_FILE, { autoSave: false });
    }
    return store;
  }

  function syncStatus() {
    status.value = password.value ? "connected" : "idle";
    connectedAt.value = password.value ? Date.now() : null;
  }

  /** Restore URL + username from `settings.json` and the password from the keychain. */
  async function hydrate() {
    if (hydrated.value) return;
    try {
      const settingsFile = await settings();
      url.value = (await settingsFile.get<string>(URL_KEY)) ?? "";
      username.value = (await settingsFile.get<string>(USERNAME_KEY)) ?? "";
      persistedUsername = username.value;

      const result = await invoke<KeychainStatus>("load_webdav_password", { username: username.value });
      keychainAvailable.value = result.available;
      if (result.password) {
        password.value = result.password;
      }
      syncStatus();
    } catch (cause) {
      error.value = describeError(cause);
      status.value = "error";
    } finally {
      hydrated.value = true;
    }
  }

  /**
   * Write the form to its storage.
   *
   * URL + username go to `settings.json`; the password goes to the OS keychain
   * (an empty password removes the stored credential). Returns a warning string
   * when the keychain is unreachable so the caller can still report success for
   * the other fields.
   */
  async function persist(): Promise<{ warning: string | null }> {
    saving.value = true;
    error.value = "";
    let warning: string | null = null;

    try {
      const settingsFile = await settings();
      await settingsFile.set(URL_KEY, url.value);
      await settingsFile.set(USERNAME_KEY, username.value);
      await settingsFile.save();

      const result = await invoke<KeychainStatus>("save_webdav_password", {
        username: username.value,
        previousUsername: persistedUsername || null,
        password: password.value,
      });

      keychainAvailable.value = result.available;
      if (!result.available) {
        warning = result.warning ?? "keychain-unavailable";
      } else {
        persistedUsername = username.value;
        syncStatus();
      }
    } catch (cause) {
      error.value = describeError(cause);
      status.value = "error";
    } finally {
      saving.value = false;
    }

    return { warning };
  }

  function disconnect() {
    status.value = "idle";
    error.value = "";
    connectedAt.value = null;
  }

  return {
    url,
    username,
    password,
    status,
    error,
    connectedAt,
    hydrated,
    keychainAvailable,
    saving,
    isConnected,
    isBusy,
    hydrate,
    persist,
    disconnect,
  };
});
