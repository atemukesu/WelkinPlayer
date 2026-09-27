import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { load, type Store } from "@tauri-apps/plugin-store";
import { describeError, invoke } from "../api";

export type WebdavStatus = "idle" | "connecting" | "connected" | "error";
export type UrlSafety = "secure" | "insecurePrivate" | "insecurePublic";

/** Mirrors the `KeychainStatus` struct returned by the Rust commands. */
interface KeychainStatus {
  available: boolean;
  hasPassword: boolean;
  warning: string | null;
}

/** Mirrors the `UrlRisk` struct returned by `webdav_url_risk`. */
export interface UrlRisk {
  safety: UrlSafety;
  host: string;
  blocked: boolean;
  allowInsecure: boolean;
}

const SETTINGS_FILE = "settings.json";
const URL_KEY = "webdav.url";
const USERNAME_KEY = "webdav.username";
const ALLOW_INSECURE_KEY = "webdav.allowInsecure";

export const useWebdavStore = defineStore("webdav", () => {
  const url = ref("");
  const username = ref("");
  /**
   * Password typed into the form. It is written to (or removed from) the OS
   * keychain by [`persist`] and then cleared from memory — never written to
   * `settings.json` and never read back from the backend.
   */
  const password = ref("");
  /** Whether a credential is currently held in the OS keychain. */
  const hasStoredPassword = ref(false);
  /** Set when the user edits the password field, so an empty value means "clear". */
  const passwordTouched = ref(false);
  /** Opt-in that suppresses the plaintext-HTTP warning and unblocks public HTTP. */
  const allowInsecure = ref(false);
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
    const connected = hasStoredPassword.value;
    status.value = connected ? "connected" : "idle";
    connectedAt.value = connected ? Date.now() : null;
  }

  /** Mark that the user edited the password field. */
  function markPasswordTouched() {
    passwordTouched.value = true;
  }

  /** Ask the backend to remove the stored credential on the next save. */
  function clearPassword() {
    password.value = "";
    passwordTouched.value = true;
  }

  /**
   * Restore URL + username from `settings.json` and only the *presence* of a
   * password from the keychain. The secret never enters the webview.
   */
  async function hydrate() {
    if (hydrated.value) return;
    try {
      const settingsFile = await settings();
      url.value = (await settingsFile.get<string>(URL_KEY)) ?? "";
      username.value = (await settingsFile.get<string>(USERNAME_KEY)) ?? "";
      allowInsecure.value = (await settingsFile.get<boolean>(ALLOW_INSECURE_KEY)) ?? false;
      persistedUsername = username.value;

      const result = await invoke<KeychainStatus>("load_webdav_password", { username: username.value });
      keychainAvailable.value = result.available;
      hasStoredPassword.value = result.hasPassword;
      syncStatus();
    } catch (cause) {
      error.value = describeError(cause);
      status.value = "error";
    } finally {
      hydrated.value = true;
    }
  }

  /** Classify a URL so the UI can warn about / block plaintext HTTP. */
  async function checkUrlRisk(target = url.value): Promise<UrlRisk | null> {
    if (!target.trim()) return null;
    return invoke<UrlRisk>("webdav_url_risk", { url: target });
  }

  /**
   * Write the form to its storage.
   *
   * URL + username + the insecure-connection opt-in go to `settings.json`; the
   * password goes to the OS keychain. The password argument is:
   * * the typed value, when the field is non-empty;
   * * `""` to clear it, when the user emptied a previously stored password;
   * * `null` to keep it, when the field was left untouched.
   *
   * Returns a warning string when the keychain is unreachable so the caller can
   * still report success for the other fields.
   */
  async function persist(): Promise<{ warning: string | null }> {
    saving.value = true;
    error.value = "";
    let warning: string | null = null;

    try {
      const settingsFile = await settings();
      await settingsFile.set(URL_KEY, url.value);
      await settingsFile.set(USERNAME_KEY, username.value);
      await settingsFile.set(ALLOW_INSECURE_KEY, allowInsecure.value);
      await settingsFile.save();

      // An untouched field means "keep whatever is already stored". Sending ""
      // here would delete a credential the keychain may still hold, which is how
      // a failed keychain read used to wipe a perfectly valid password.
      const passwordArg = password.value
        ? password.value
        : passwordTouched.value
          ? ""
          : null;

      const result = await invoke<KeychainStatus>("save_webdav_password", {
        username: username.value,
        previousUsername: persistedUsername || null,
        password: passwordArg,
      });

      keychainAvailable.value = result.available;
      if (!result.available) {
        warning = result.warning ?? "keychain-unavailable";
      } else {
        hasStoredPassword.value = result.hasPassword;
        persistedUsername = username.value;
        // The secret now lives in the keychain; drop it from the webview.
        password.value = "";
        passwordTouched.value = false;
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
    hasStoredPassword,
    passwordTouched,
    allowInsecure,
    status,
    error,
    connectedAt,
    hydrated,
    keychainAvailable,
    saving,
    isConnected,
    isBusy,
    hydrate,
    checkUrlRisk,
    markPasswordTouched,
    clearPassword,
    persist,
    disconnect,
  };
});
