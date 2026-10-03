import type { BrowserObservation } from "./api/core-foundation";
export type ObservationPage = BrowserObservation["page"];
const MAX_EVENTS = 8;
let page: ObservationPage | null = null;
let enabled = false;
let events: BrowserObservation[] = [];
let dropped = false;
let lastAttemptAt = 0;
const listeners = new Set<() => void>();
export function subscribeBrowserObservation(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function browserObservationEnabled(): boolean {
  return enabled;
}
export function configureBrowserObservation(
  current: ObservationPage | null,
  active: boolean,
): void {
  page = current;
  const changed = enabled !== active;
  enabled = active;
  if (!active) {
    events = [];
    dropped = false;
  }
  if (changed) for (const listener of listeners) listener();
}
function device(): BrowserObservation["device"] {
  return window.matchMedia?.("(max-width: 768px)").matches ? "mobile" : "desktop";
}
export function observeBrowser(
  kind: BrowserObservation["kind"],
  seconds = 0,
  explicitPage = page,
  outcome?: BrowserObservation["outcome"],
): void {
  if (!enabled || !explicitPage || !Number.isFinite(seconds) || seconds < 0 || seconds > 3600)
    return;
  const event: BrowserObservation = {
    page: explicitPage,
    device: device(),
    kind,
    valueSeconds: seconds,
    ...(outcome ? { outcome } : {}),
  };
  if (events.length >= MAX_EVENTS) {
    if (kind === "data_ready" || kind === "update_to_paint") {
      const replace = events.findIndex(
        (event) => event.kind === "api_request" || event.kind === "long_task",
      );
      if (replace >= 0) events.splice(replace, 1);
      else {
        dropped = true;
        return;
      }
    } else {
      dropped = true;
      return;
    }
  }
  events.push(event);
}
export async function flushBrowserObservations(
  send: (events: BrowserObservation[]) => Promise<void>,
  now = Date.now(),
): Promise<void> {
  if (!enabled || now - lastAttemptAt < 60_000 || events.length === 0) return;
  lastAttemptAt = now; // Failed and rejected batches have the same frequency ceiling.
  const batch = events.splice(0, MAX_EVENTS);
  if (
    navigator.onLine === false ||
    new TextEncoder().encode(JSON.stringify({ events: batch })).byteLength > 2048
  ) {
    dropped = true;
    return;
  }
  try {
    await send(batch);
  } catch {
    dropped = true;
  }
  if (dropped) {
    dropped = false;
    observeBrowser("dropped");
  }
}
export function observeSseLifecycle(source: EventSource): () => void {
  const ownerPage = page;
  let started: number | null = null;
  const open = (): void => {
    started = performance.now();
  };
  const end = (outcome: BrowserObservation["outcome"]): void => {
    if (started === null) return;
    observeBrowser("sse", (performance.now() - started) / 1000, ownerPage, outcome);
    started = null;
  };
  const error = (): void => end("error");
  source.addEventListener("open", open);
  source.addEventListener("error", error);
  return (): void => {
    end("normal");
    source.removeEventListener("open", open);
    source.removeEventListener("error", error);
  };
}
