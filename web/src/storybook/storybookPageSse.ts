import type { BroadcastPayload } from "../lib/api";
import type { SubscriptionTopicEnvelope } from "../lib/sse";

export type StorybookPageSsePayload = BroadcastPayload | SubscriptionTopicEnvelope<unknown>;

export interface StorybookPageSseController {
  emit: (payload: StorybookPageSsePayload) => void;
  emitOpen: () => void;
  reset: () => void;
}

declare global {
  interface Window {
    __storybookPageSseController__?: StorybookPageSseController;
  }
}

export function getStorybookPageSseController(): StorybookPageSseController | null {
  if (typeof window === "undefined") {
    return null;
  }
  return window.__storybookPageSseController__ ?? null;
}
