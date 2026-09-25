import { invoke as tauriInvoke } from "@tauri-apps/api/core";

/** Shape of the `{ code, message }` payload produced by the Rust `AppError`. */
export interface AppErrorPayload {
  code: string;
  message: string;
}

/** Structured error thrown by {@link invoke}. */
export class AppError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "AppError";
    this.code = code;
  }
}

/**
 * Normalize anything thrown by Tauri (or the network layer) into an
 * [`AppError`] so callers always get a `code` and a readable `message`.
 */
export function toAppError(error: unknown): AppError {
  if (error instanceof AppError) return error;

  if (typeof error === "string") {
    return new AppError("UNKNOWN", error);
  }

  if (error && typeof error === "object") {
    const { code, message } = error as Partial<AppErrorPayload>;
    if (typeof message === "string") {
      return new AppError(typeof code === "string" ? code : "UNKNOWN", message);
    }
  }

  return new AppError("UNKNOWN", String(error));
}

/**
 * Thin wrapper around Tauri's `invoke` that guarantees a structured
 * [`AppError`] on failure and logs it in the browser console.
 *
 * Use this instead of importing `invoke` from `@tauri-apps/api/core` directly.
 */
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    const appError = toAppError(error);
    console.error(`[welkin] command "${command}" failed [${appError.code}]: ${appError.message}`);
    throw appError;
  }
}

/** Render an unknown error as a single user-readable line. */
export function describeError(error: unknown): string {
  const appError = toAppError(error);
  return `${appError.code} · ${appError.message}`;
}
