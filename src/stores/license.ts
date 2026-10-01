import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { invoke } from "../api";

/**
 * Pro activation state as reported by the Rust backend.
 *
 * Verification lives entirely in Rust; this store only mirrors the result so
 * the UI can render the edition badge and activation status.
 */
export interface ProStatus {
  active: boolean;
  signer: string | null;
  tier: number | null;
  username: string | null;
  platform: string | null;
  expiresAt: number | null;
  error: string | null;
}

const INACTIVE: ProStatus = {
  active: false,
  signer: null,
  tier: null,
  username: null,
  platform: null,
  expiresAt: null,
  error: null,
};

export const useLicenseStore = defineStore("license", () => {
  const status = ref<ProStatus>({ ...INACTIVE });

  /** Whether a valid, unexpired, device-bound Pro code is active. */
  const isPro = computed(() => status.value.active);

  /** Load and re-verify any stored activation code. */
  async function refresh() {
    try {
      status.value = await invoke<ProStatus>("get_pro_status");
    } catch {
      // A failed probe must never be treated as activation.
    }
  }

  /** Verify and persist a code; throws a structured AppError on failure. */
  async function activate(code: string): Promise<ProStatus> {
    status.value = await invoke<ProStatus>("activate_pro", { code });
    return status.value;
  }

  return { status, isPro, refresh, activate };
});
