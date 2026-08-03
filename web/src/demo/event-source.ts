import type { SubscriptionTopicDescriptor } from "../lib/sse";
import { subscribeToDemoRealtime } from "./events";
import {
  DEMO_SCHEMA_EPOCH,
  demoTopicDescriptorKey,
  parseDemoRequestedTopics,
  resolveDemoTopicPayload,
} from "./topic-payloads";

const DEMO_EVENT_SOURCE_KEY = Symbol.for("codex-vibe-monitor.demo.event-source");

type DemoEventSourceWindow = Window & {
  [DEMO_EVENT_SOURCE_KEY]?: boolean;
};

declare global {
  interface Window {
    __CVM_DEMO_CREATE_EVENT_SOURCE__?: (path: string) => EventSource;
  }
}

export class DemoTopicEventSource implements EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  readonly CONNECTING = DemoTopicEventSource.CONNECTING;
  readonly OPEN = DemoTopicEventSource.OPEN;
  readonly CLOSED = DemoTopicEventSource.CLOSED;
  readonly url: string;
  readonly withCredentials = false;
  readyState = DemoTopicEventSource.CONNECTING;
  onerror: ((this: EventSource, event: Event) => unknown) | null = null;
  onmessage: ((this: EventSource, event: MessageEvent<string>) => unknown) | null = null;
  onopen: ((this: EventSource, event: Event) => unknown) | null = null;

  #cursor = 1;
  #listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();
  #unsubscribe: (() => void) | null = null;

  constructor(url: string | URL) {
    this.url = new URL(url.toString(), window.location.href).toString();
    window.setTimeout(() => void this.connect(), 0);
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const listeners = this.#listeners.get(type) ?? new Set<EventListenerOrEventListenerObject>();
    listeners.add(listener);
    this.#listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    this.#listeners.get(type)?.delete(listener);
  }

  dispatchEvent(event: Event) {
    this.emit(event.type, event);
    return true;
  }

  close() {
    this.readyState = DemoTopicEventSource.CLOSED;
    this.#unsubscribe?.();
    this.#unsubscribe = null;
  }

  private async connect() {
    if (this.readyState === DemoTopicEventSource.CLOSED) return;
    const topics = parseDemoRequestedTopics(this.url);
    this.readyState = DemoTopicEventSource.OPEN;
    this.emit("open", new Event("open"));
    await this.publish(topics, "snapshot");
    if (this.readyState !== DemoTopicEventSource.OPEN) return;
    this.#unsubscribe = subscribeToDemoRealtime(() => void this.publish(topics, "live"));
  }

  private async publish(topics: SubscriptionTopicDescriptor[], type: "snapshot" | "live") {
    for (const topic of topics) {
      if (this.readyState !== DemoTopicEventSource.OPEN) return;
      const payload = await resolveDemoTopicPayload(topic, this.url);
      if (payload == null) continue;
      this.emit(
        "message",
        new MessageEvent("message", {
          data: JSON.stringify({
            type,
            topic,
            topicKey: demoTopicDescriptorKey(topic),
            schemaEpoch: DEMO_SCHEMA_EPOCH,
            cursor: this.#cursor++,
            payload,
          }),
        }),
      );
    }
  }

  private emit(type: string, event: Event) {
    if (type === "open") this.onopen?.call(this as unknown as EventSource, event);
    if (type === "error") this.onerror?.call(this as unknown as EventSource, event);
    if (type === "message") {
      this.onmessage?.call(this as unknown as EventSource, event as MessageEvent<string>);
    }
    for (const listener of this.#listeners.get(type) ?? []) {
      if (typeof listener === "function") {
        listener(event);
      } else {
        listener.handleEvent(event);
      }
    }
  }
}

export function isDemoTopicEventSourcePath(requestPath: string, baseUrl: string) {
  const requestUrl = new URL(requestPath, "http://demo.invalid");
  const eventsUrl = new URL("events", new URL(baseUrl, "http://demo.invalid"));
  return requestUrl.pathname === eventsUrl.pathname;
}

export function installDemoEventSource() {
  if (typeof window === "undefined") return;
  const demoWindow = window as DemoEventSourceWindow;
  if (demoWindow[DEMO_EVENT_SOURCE_KEY]) return;

  const NativeEventSource = window.EventSource;
  window.__CVM_DEMO_CREATE_EVENT_SOURCE__ = (path) => {
    const requestUrl = new URL(path, window.location.href);
    if (isDemoTopicEventSourcePath(requestUrl.toString(), import.meta.env.BASE_URL)) {
      return new DemoTopicEventSource(requestUrl) as unknown as EventSource;
    }
    return new NativeEventSource(path);
  };
  demoWindow[DEMO_EVENT_SOURCE_KEY] = true;
}
