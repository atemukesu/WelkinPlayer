import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { load, type Store } from "@tauri-apps/plugin-store";
import { describeError, invoke } from "../api";
import { createId } from "../lib/profile";
import { isCloudSource, sourceLabel } from "../lib/sources";
import type { SongSource, SourceKind } from "../lib/sources";

const SETTINGS_FILE = "settings.json";
const SOURCES_KEY = "sources";
const SYNC_SOURCE_KEY = "sources.syncId";
const SCHEMA_KEY = "sources.schema";
const SCHEMA_VERSION = 1;

/** Mirrors the Rust `KeychainStatus`. */
interface KeychainStatus {
  available: boolean;
  hasPassword: boolean;
  warning: string | null;
}

/** Mirrors the Rust `UrlRisk`. */
export interface UrlRisk {
  safety: "secure" | "insecurePrivate" | "insecurePublic";
  host: string;
  blocked: boolean;
  allowInsecure: boolean;
}

/**
 * Holds the configured song sources and the id of the source used to store the
 * synced profile / playback state. CRUD is written straight to `settings.json`;
 * the backend reads the same keys.
 */
export const useSourcesStore = defineStore("sources", () => {
  const sources = ref<SongSource[]>([]);
  const syncId = ref<string | null>(null);
  const hydrated = ref(false);
  const saving = ref(false);
  const error = ref("");
  /** Whether a WebDAV password is stored for each source id. */
  const passwords = ref<Record<string, boolean>>({});
  const keychainAvailable = ref(true);
  /** Bumped when the sources that affect the library change, so it can reload. */
  const revision = ref(0);

  let store: Store | null = null;

  const hasSources = computed(() => sources.value.length > 0);
  const cloudSources = computed(() => sources.value.filter(isCloudSource));
  /** Whether any source can store data in the cloud (a WebDAV source exists). */
  const hasCloudSync = computed(() => cloudSources.value.length > 0);
  /** The source currently used to store synced data, when configured. */
  const syncSource = computed(() => sources.value.find((source) => source.id === syncId.value) ?? null);
  /** Whether the sync source is local-only (no cloud possible). */
  const syncIsLocal = computed(() => !!syncSource.value && syncSource.value.kind === "local");

  async function settings() {
    if (!store) store = await load(SETTINGS_FILE, { autoSave: false });
    return store;
  }

  function label(source: SongSource): string {
    return sourceLabel(source);
  }

  /** Load the source list from `settings.json`. */
  async function hydrate() {
    if (hydrated.value) return;
    try {
      const file = await settings();
      const stored = (await file.get<SongSource[]>(SOURCES_KEY)) ?? [];
      sources.value = stored.filter((item) => item && typeof item.id === "string");
      syncId.value = (await file.get<string>(SYNC_SOURCE_KEY)) ?? null;
      if (syncId.value && !sources.value.some((source) => source.id === syncId.value)) {
        syncId.value = null;
      }
      await refreshPasswordStatuses();
    } catch (cause) {
      error.value = describeError(cause);
    } finally {
      hydrated.value = true;
    }
  }

  async function refreshPasswordStatuses() {
    const next: Record<string, boolean> = {};
    for (const source of sources.value) {
      if (source.kind !== "webdav") continue;
      try {
        const status = await invoke<KeychainStatus>("load_source_password", { sourceId: source.id });
        keychainAvailable.value = status.available;
        next[source.id] = status.hasPassword;
      } catch {
        next[source.id] = false;
      }
    }
    passwords.value = next;
  }

  /** Write the current list + sync selection to `settings.json`. */
  async function persist() {
    saving.value = true;
    error.value = "";
    try {
      const file = await settings();
      await file.set(SOURCES_KEY, JSON.parse(JSON.stringify(sources.value)));
      await file.set(SYNC_SOURCE_KEY, syncId.value ?? "");
      await file.set(SCHEMA_KEY, SCHEMA_VERSION);
      await file.save();
    } catch (cause) {
      error.value = describeError(cause);
    } finally {
      saving.value = false;
    }
  }

  /** A display name that never collides with another source's name. */
  function uniqueName(base: string, excludeId?: string): string {
    const trimmed = base.trim() || "Source";
    const taken = new Set(
      sources.value
        .filter((source) => source.id !== excludeId)
        .map((source) => source.name.trim()),
    );
    if (!taken.has(trimmed)) return trimmed;
    let index = 2;
    while (taken.has(`${trimmed} ${index}`)) index += 1;
    return `${trimmed} ${index}`;
  }

  /** Build a fresh source of the requested kind with a stable id. */
  function draft(kind: SourceKind): SongSource {
    return {
      id: createId(),
      kind,
      name: uniqueName(kind === "local" ? "本地音乐" : "WebDAV"),
      url: kind === "webdav" ? "" : undefined,
      username: kind === "webdav" ? "" : undefined,
      allowInsecure: kind === "webdav" ? false : undefined,
      rootPath: kind === "local" ? "" : undefined,
    };
  }

  async function addSource(source: SongSource) {
    sources.value = [...sources.value, source];
    const current = sources.value.find((item) => item.id === syncId.value);
    // Auto-pick a sync location: use the new source when none is set, or
    // upgrade a local-only selection to a newly added cloud source, so the
    // user never has to select it manually.
    if (!current || (isCloudSource(source) && current.kind === "local")) {
      syncId.value = source.id;
    }
    revision.value += 1;
    await persist();
  }

  async function updateSource(id: string, patch: Partial<SongSource>) {
    const next: Partial<SongSource> = { ...patch };
    if (next.name !== undefined) next.name = uniqueName(next.name, id);
    sources.value = sources.value.map((source) => (source.id === id ? { ...source, ...next } : source));
    // Only connection-relevant edits change what the library should load.
    if ("url" in patch || "username" in patch || "rootPath" in patch || "allowInsecure" in patch || "kind" in patch) {
      revision.value += 1;
    }
    await persist();
  }

  async function removeSource(id: string) {
    sources.value = sources.value.filter((source) => source.id !== id);
    if (syncId.value === id) {
      syncId.value = cloudSources.value[0]?.id ?? sources.value[0]?.id ?? null;
    }
    await invoke("delete_source_password", { sourceId: id }).catch(() => {});
    revision.value += 1;
    await persist();
  }

  async function setSyncId(id: string | null) {
    syncId.value = id;
    await persist();
  }

  /** Store (or clear) a WebDAV password for a source. */
  async function setPassword(id: string, password: string): Promise<string | null> {
    try {
      const status = await invoke<KeychainStatus>("save_source_password", { sourceId: id, password });
      keychainAvailable.value = status.available;
      passwords.value = { ...passwords.value, [id]: status.hasPassword };
      // A newly saved credential can make the source readable: reload.
      revision.value += 1;
      return status.available ? null : status.warning ?? "keychain-unavailable";
    } catch (cause) {
      error.value = describeError(cause);
      return describeError(cause);
    }
  }

  async function clearPassword(id: string) {
    await setPassword(id, "");
  }

  function hasPassword(id: string): boolean {
    return !!passwords.value[id];
  }

  async function checkUrlRisk(url: string, allowInsecure: boolean): Promise<UrlRisk | null> {
    if (!url.trim()) return null;
    try {
      return await invoke<UrlRisk>("webdav_url_risk", { url, allowInsecure });
    } catch {
      return null;
    }
  }

  async function testConnection(id: string): Promise<string> {
    return invoke<string>("test_source_connection", { sourceId: id });
  }

  async function testWrite(id: string): Promise<string> {
    return invoke<string>("test_source_write", { sourceId: id });
  }

  async function pickFolder(): Promise<string | null> {
    return invoke<string | null>("pick_local_folder");
  }

  return {
    sources,
    syncId,
    hydrated,
    saving,
    error,
    passwords,
    keychainAvailable,
    revision,
    hasSources,
    cloudSources,
    hasCloudSync,
    syncSource,
    syncIsLocal,
    hydrate,
    persist,
    draft,
    addSource,
    updateSource,
    removeSource,
    setSyncId,
    setPassword,
    clearPassword,
    hasPassword,
    refreshPasswordStatuses,
    checkUrlRisk,
    testConnection,
    testWrite,
    pickFolder,
    label,
  };
});
