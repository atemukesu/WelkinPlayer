import { onBeforeUnmount, onMounted, watch } from "vue";

/** Minimal typing for the Screen Wake Lock API, which is not in every TS DOM lib. */
interface WakeLockSentinelLike {
  release(): Promise<void>;
  addEventListener(type: "release", listener: () => void): void;
}

interface WakeLockLike {
  request(type: "screen"): Promise<WakeLockSentinelLike>;
}

function wakeLockApi(): WakeLockLike | null {
  if (typeof navigator === "undefined") return null;
  return (navigator as Navigator & { wakeLock?: WakeLockLike }).wakeLock ?? null;
}

/**
 * Keep the screen awake while the calling view is mounted, or while `active`
 * returns true. The browser drops the lock whenever the page is hidden, so it
 * is re-acquired on `visibilitychange`. Safe to call where the API is missing:
 * the request is simply skipped.
 */
export function useWakeLock(active: () => boolean = () => true) {
  let sentinel: WakeLockSentinelLike | null = null;
  let disposed = false;

  async function acquire() {
    if (disposed || sentinel || !active()) return;
    const api = wakeLockApi();
    if (!api) return;
    try {
      const lock = await api.request("screen");
      if (disposed) {
        void lock.release().catch(() => {});
        return;
      }
      sentinel = lock;
      lock.addEventListener("release", () => { sentinel = null; });
    } catch {
      sentinel = null;
    }
  }

  function release() {
    const lock = sentinel;
    sentinel = null;
    if (lock) void lock.release().catch(() => {});
  }

  function onVisibility() {
    if (document.visibilityState === "visible") void acquire();
  }

  onMounted(() => {
    void acquire();
    document.addEventListener("visibilitychange", onVisibility);
  });

  onBeforeUnmount(() => {
    disposed = true;
    document.removeEventListener("visibilitychange", onVisibility);
    release();
  });

  watch(active, (value) => {
    if (value) void acquire();
    else release();
  });
}
