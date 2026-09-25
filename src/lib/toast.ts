import { reactive } from "vue";

export type ToastType = "success" | "error" | "warning" | "info";

export interface Toast {
  id: number;
  type: ToastType;
  message: string;
}

const toasts = reactive<Toast[]>([]);
let nextId = 1;

/** Show a transient notification. Returns its id so it can be dismissed early. */
export function pushToast(type: ToastType, message: string, duration = 3400) {
  const id = nextId++;
  toasts.push({ id, type, message });
  if (duration > 0) {
    window.setTimeout(() => dismissToast(id), duration);
  }
  return id;
}

export function dismissToast(id: number) {
  const index = toasts.findIndex((toast) => toast.id === id);
  if (index !== -1) toasts.splice(index, 1);
}

export { toasts };
