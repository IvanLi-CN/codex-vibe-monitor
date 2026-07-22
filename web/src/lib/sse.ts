import { createEventSource } from "./api";

declare global {
  interface Window {
    __CVM_SSE__?: {
      requestImmediateReconnect: typeof requestImmediateReconnect;
      getCurrentSseDiagnostics: typeof getCurrentSseDiagnostics;
      getCurrentSseStatus: typeof getCurrentSseStatus;
    };
  }
}

export type SseConnectionPhase = "idle" | "connecting" | "reconnecting" | "connected" | "disabled";
export type SseReconnectReason =
  | "initial"
  | "topic-change"
  | "topic-refresh"
  | "manual"
  | "eventsource-error"
  | "watchdog-closed"
  | "watchdog-timeout"
  | "visibility-visible";
export type SseTerminalOutcome =
  | "idle"
  | "open"
  | "topic-change"
  | "eventsource-error"
  | "watchdog-closed"
  | "watchdog-timeout"
  | "disabled"
  | "unsupported"
  | "cleanup";

export interface SseStatus {
  phase: SseConnectionPhase;
  downtimeMs: number;
  nextRetryAt: number | null;
  autoReconnect: boolean;
}

export interface SseDiagnostics {
  attempt: number | null;
  reason: SseReconnectReason | null;
  activeTopics: string[];
  resumeTopics: string[];
  forcedSnapshotTopics: string[];
  lastMessageAt: number | null;
  lastOpenAt: number | null;
  lastErrorAt: number | null;
  lastConnectionStartedAt: number | null;
  lastTerminalOutcome: SseTerminalOutcome | null;
}

export interface SubscriptionTopicDescriptor {
  topic: string;
  params?: Record<string, string | number | boolean | null | undefined>;
}

export interface SubscriptionResumeQueryCursor {
  topicIndex: number;
  cursor: number;
  schemaEpoch: string;
}

export interface SubscriptionTopicEnvelope<T = unknown> {
  type: "snapshot" | "replay" | "live";
  topic: SubscriptionTopicDescriptor;
  topicKey: string;
  schemaEpoch: string;
  cursor: number;
  payload: T;
}

export interface SubscriptionTopicState<T = unknown> {
  descriptor: SubscriptionTopicDescriptor;
  topicKey: string | null;
  schemaEpoch: string | null;
  cursor: number | null;
  payload: T | null;
  lastKind: SubscriptionTopicEnvelope["type"] | null;
  receivedAt: number | null;
}

type TopicListener<T = unknown> = (event: SubscriptionTopicEnvelope<T>) => void;
type StatusListener = (status: SseStatus) => void;
type DiagnosticsListener = (diagnostics: SseDiagnostics) => void;
type OpenListener = () => void;
type ActivityListener = () => void;

type TopicEntry = {
  descriptor: SubscriptionTopicDescriptor;
  listeners: Set<TopicListener>;
};

type TopicCacheEntry = SubscriptionTopicState;

let eventSource: EventSource | null = null;
const topicEntries = new Map<string, TopicEntry>();
const topicCache = new Map<string, TopicCacheEntry>();
const openListeners = new Set<OpenListener>();
const activityListeners = new Set<ActivityListener>();
const statusListeners = new Set<StatusListener>();
const diagnosticsListeners = new Set<DiagnosticsListener>();
const forcedSnapshotDescriptors = new Set<string>();

let reconnectTimer: number | null = null;
let connectionWatchdog: number | null = null;
let reconnectAttempts = 0;
let connectingSince: number | null = null;
let downtimeStartedAt: number | null = null;
let downtimeTicker: number | null = null;
let nextRetryAt: number | null = null;
let hasConnectedOnce = false;
let sseDisabled = false;
let connectionPhase: SseConnectionPhase = "idle";
let lastStatus: SseStatus = {
  phase: "idle",
  downtimeMs: 0,
  nextRetryAt: null,
  autoReconnect: true,
};
let lastDiagnostics: SseDiagnostics = {
  attempt: null,
  reason: null,
  activeTopics: [],
  resumeTopics: [],
  forcedSnapshotTopics: [],
  lastMessageAt: null,
  lastOpenAt: null,
  lastErrorAt: null,
  lastConnectionStartedAt: null,
  lastTerminalOutcome: null,
};
let activeConnectionSignature = "";
let connectionAttemptCounter = 0;
let nextConnectReason: SseReconnectReason = "initial";

const BASE_RECONNECT_DELAY_MS = 2_000;
const MAX_RECONNECT_DELAY_MS = 30_000;
const CONNECTING_TIMEOUT_MS = 45_000;
const WATCHDOG_INTERVAL_MS = 5_000;
const MAX_DOWNTIME_BEFORE_DISABLE_MS = 10 * 60 * 1000;

function normalizeTopicParams(
  params?: Record<string, string | number | boolean | null | undefined>,
) {
  const normalized = Object.entries(params ?? {})
    .filter(([, value]) => value != null && `${value}`.trim() !== "")
    .map(([key, value]) => [key, `${value}`] as const)
    .sort(([left], [right]) => left.localeCompare(right));
  return Object.fromEntries(normalized);
}

export function buildTopicDescriptor(
  topic: string,
  params?: Record<string, string | number | boolean | null | undefined>,
): SubscriptionTopicDescriptor {
  return {
    topic: topic.trim(),
    params: normalizeTopicParams(params),
  };
}

export function getTopicDescriptorKey(descriptor: SubscriptionTopicDescriptor) {
  return JSON.stringify({
    topic: descriptor.topic.trim(),
    params: normalizeTopicParams(descriptor.params),
  });
}

function normalizeIncomingDescriptor(
  raw: SubscriptionTopicDescriptor,
): SubscriptionTopicDescriptor {
  return buildTopicDescriptor(raw.topic, raw.params);
}

function encodeBase64Url(raw: string) {
  const bytes = new TextEncoder().encode(raw);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function formatTopicLabel(descriptor: SubscriptionTopicDescriptor) {
  const normalized = normalizeTopicParams(descriptor.params);
  const query = new URLSearchParams(normalized).toString();
  return query ? `${descriptor.topic.trim()}?${query}` : descriptor.topic.trim();
}

function getActiveDescriptors() {
  const descriptors = Array.from(topicEntries.values())
    .map((entry) => entry.descriptor)
    .sort((left, right) => getTopicDescriptorKey(left).localeCompare(getTopicDescriptorKey(right)));
  return descriptors;
}

function getResumeDescriptors(descriptors = getActiveDescriptors()) {
  return descriptors.filter((descriptor) => {
    const descriptorKey = getTopicDescriptorKey(descriptor);
    if (forcedSnapshotDescriptors.has(descriptorKey)) {
      return false;
    }
    const cached = topicCache.get(descriptorKey);
    return !!cached?.topicKey && cached.cursor != null && !!cached.schemaEpoch;
  });
}

function getForcedSnapshotDescriptors(descriptors = getActiveDescriptors()) {
  return descriptors.filter((descriptor) =>
    forcedSnapshotDescriptors.has(getTopicDescriptorKey(descriptor)),
  );
}

function buildConnectionRequest(attempt: number, reason: SseReconnectReason) {
  const descriptors = getActiveDescriptors();
  if (descriptors.length === 0) {
    return null;
  }

  const resumeTopics: string[] = [];
  const forcedSnapshotTopics = getForcedSnapshotDescriptors(descriptors).map(formatTopicLabel);
  const resume = descriptors.flatMap((descriptor, topicIndex) => {
    const descriptorKey = getTopicDescriptorKey(descriptor);
    if (forcedSnapshotDescriptors.has(descriptorKey)) {
      return [];
    }
    const cached = topicCache.get(descriptorKey);
    if (!cached?.topicKey || cached.cursor == null || !cached.schemaEpoch) {
      return [];
    }
    resumeTopics.push(formatTopicLabel(descriptor));
    return [
      {
        topicIndex,
        cursor: cached.cursor,
        schemaEpoch: cached.schemaEpoch,
      } satisfies SubscriptionResumeQueryCursor,
    ];
  });

  const search = new URLSearchParams();
  search.set("topics", encodeBase64Url(JSON.stringify(descriptors)));
  if (resume.length > 0) {
    search.set("resume", encodeBase64Url(JSON.stringify(resume)));
  }
  search.set("attempt", `${attempt}`);
  search.set("reason", reason);
  return {
    path: `/events?${search.toString()}`,
    activeTopics: descriptors.map(formatTopicLabel),
    resumeTopics,
    forcedSnapshotTopics,
  };
}

function computeConnectionSignature() {
  return Array.from(topicEntries.values())
    .map((entry) => getTopicDescriptorKey(entry.descriptor))
    .sort((left, right) => left.localeCompare(right))
    .join("|");
}

function isEventSourceSupported() {
  return typeof EventSource !== "undefined";
}

function hasActiveTopicSubscribers() {
  return topicEntries.size > 0;
}

function computeStatus(): SseStatus {
  const now = Date.now();
  const downtime = downtimeStartedAt != null ? now - downtimeStartedAt : 0;
  return {
    phase: connectionPhase,
    downtimeMs: downtime,
    nextRetryAt,
    autoReconnect: !sseDisabled,
  };
}

function arraysEqual(left: string[], right: string[]) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function diagnosticsEqual(left: SseDiagnostics, right: SseDiagnostics) {
  return (
    left.attempt === right.attempt &&
    left.reason === right.reason &&
    left.lastMessageAt === right.lastMessageAt &&
    left.lastOpenAt === right.lastOpenAt &&
    left.lastErrorAt === right.lastErrorAt &&
    left.lastConnectionStartedAt === right.lastConnectionStartedAt &&
    left.lastTerminalOutcome === right.lastTerminalOutcome &&
    arraysEqual(left.activeTopics, right.activeTopics) &&
    arraysEqual(left.resumeTopics, right.resumeTopics) &&
    arraysEqual(left.forcedSnapshotTopics, right.forcedSnapshotTopics)
  );
}

function emitDiagnostics(patch: Partial<SseDiagnostics> = {}) {
  const nextDiagnostics = {
    ...lastDiagnostics,
    activeTopics: getActiveDescriptors().map(formatTopicLabel),
    resumeTopics: getResumeDescriptors().map(formatTopicLabel),
    forcedSnapshotTopics: getForcedSnapshotDescriptors().map(formatTopicLabel),
    ...patch,
  };
  if (diagnosticsEqual(lastDiagnostics, nextDiagnostics)) {
    return;
  }
  lastDiagnostics = nextDiagnostics;
  diagnosticsListeners.forEach((listener) => {
    try {
      listener(lastDiagnostics);
    } catch (error) {
      console.error("Failed to dispatch SSE diagnostics update", error);
    }
  });
}

function emitStatus() {
  lastStatus = computeStatus();
  statusListeners.forEach((listener) => {
    try {
      listener(lastStatus);
    } catch (error) {
      console.error("Failed to dispatch SSE status update", error);
    }
  });
}

function setConnectionPhase(next: SseConnectionPhase) {
  if (connectionPhase !== next) {
    connectionPhase = next;
  }
  emitStatus();
}

function startDowntimeTicker() {
  if (downtimeTicker != null) return;
  downtimeTicker = window.setInterval(() => {
    emitStatus();
    if (
      downtimeStartedAt != null &&
      Date.now() - downtimeStartedAt >= MAX_DOWNTIME_BEFORE_DISABLE_MS &&
      !sseDisabled
    ) {
      disableSse();
    }
  }, 1_000);
}

function stopDowntimeTicker() {
  if (downtimeTicker != null) {
    clearInterval(downtimeTicker);
    downtimeTicker = null;
  }
}

function beginDowntimeWindow() {
  if (downtimeStartedAt == null) {
    downtimeStartedAt = Date.now();
    startDowntimeTicker();
  }
  emitStatus();
}

function resetDowntimeWindow() {
  downtimeStartedAt = null;
  stopDowntimeTicker();
  emitStatus();
}

function clearReconnectTimer() {
  if (reconnectTimer != null) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  nextRetryAt = null;
  emitStatus();
}

function stopConnectionWatchdog() {
  if (connectionWatchdog != null) {
    clearInterval(connectionWatchdog);
    connectionWatchdog = null;
  }
  connectingSince = null;
}

function destroyEventSource() {
  if (!eventSource) return;
  eventSource.removeEventListener("message", handleMessage as EventListener);
  eventSource.removeEventListener("error", handleError);
  eventSource.removeEventListener("open", handleOpen);
  eventSource.close();
  eventSource = null;
  activeConnectionSignature = "";
}

function disableSse(outcome: SseTerminalOutcome = "disabled") {
  if (sseDisabled) return;
  sseDisabled = true;
  destroyEventSource();
  stopConnectionWatchdog();
  clearReconnectTimer();
  emitDiagnostics({ lastTerminalOutcome: outcome });
  setConnectionPhase("disabled");
}

function handleMessage(event: MessageEvent<string>) {
  try {
    const raw = JSON.parse(event.data) as
      | (Partial<SubscriptionTopicEnvelope> & {
          topic_key?: string;
          schema_epoch?: string;
        })
      | null;
    if (!raw?.topic) {
      return;
    }
    const topicKey = raw.topicKey ?? raw.topic_key;
    const schemaEpoch = raw.schemaEpoch ?? raw.schema_epoch;
    if (
      (raw.type !== "snapshot" && raw.type !== "replay" && raw.type !== "live") ||
      !topicKey ||
      !schemaEpoch ||
      typeof raw.cursor !== "number"
    ) {
      return;
    }
    const payload: SubscriptionTopicEnvelope = {
      type: raw.type,
      topic: raw.topic,
      topicKey,
      schemaEpoch,
      cursor: raw.cursor,
      payload: raw.payload,
    };
    const descriptor = normalizeIncomingDescriptor(payload.topic);
    const descriptorKey = getTopicDescriptorKey(descriptor);
    const receivedAt = Date.now();
    const nextState: TopicCacheEntry = {
      descriptor,
      topicKey: payload.topicKey,
      schemaEpoch: payload.schemaEpoch,
      cursor: payload.cursor,
      payload: payload.payload,
      lastKind: payload.type,
      receivedAt,
    };
    topicCache.set(descriptorKey, nextState);
    forcedSnapshotDescriptors.delete(descriptorKey);
    emitDiagnostics({ lastMessageAt: receivedAt });
    const entry = topicEntries.get(descriptorKey);
    if (entry) {
      entry.listeners.forEach((listener) => {
        try {
          listener({
            ...payload,
            topic: descriptor,
          });
        } catch (error) {
          console.error("Failed to dispatch subscription topic event", error);
        }
      });
    }
    activityListeners.forEach((listener) => {
      try {
        listener();
      } catch {
        // ignore activity listener failures
      }
    });
  } catch (error) {
    console.error("Failed to parse subscription SSE message", error);
  }
}

function handleError() {
  if (!hasActiveTopicSubscribers()) return;
  emitDiagnostics({
    lastErrorAt: Date.now(),
    lastTerminalOutcome: "eventsource-error",
  });
  beginDowntimeWindow();
  scheduleReconnect({ reason: "eventsource-error" });
}

function handleOpen() {
  reconnectAttempts = 0;
  connectingSince = null;
  hasConnectedOnce = true;
  nextRetryAt = null;
  clearReconnectTimer();
  resetDowntimeWindow();
  emitDiagnostics({
    lastOpenAt: Date.now(),
    lastTerminalOutcome: "open",
  });
  setConnectionPhase("connected");
  openListeners.forEach((listener) => {
    try {
      listener();
    } catch {
      // ignore
    }
  });
}

function ensureEventSource() {
  if (!hasActiveTopicSubscribers()) {
    return null;
  }
  if (sseDisabled) return eventSource;
  if (!isEventSourceSupported()) {
    sseDisabled = true;
    clearReconnectTimer();
    emitDiagnostics({ lastTerminalOutcome: "unsupported" });
    setConnectionPhase("disabled");
    return null;
  }

  const signature = computeConnectionSignature();
  if (eventSource && activeConnectionSignature === signature) {
    emitDiagnostics();
    return eventSource;
  }

  const reason = nextConnectReason;
  const attempt = connectionAttemptCounter + 1;
  const request = buildConnectionRequest(attempt, reason);
  if (!request) {
    return null;
  }

  if (eventSource) {
    emitDiagnostics({ lastTerminalOutcome: "topic-change" });
  }
  destroyEventSource();
  connectionAttemptCounter = attempt;
  connectingSince = Date.now();
  lastDiagnostics = {
    ...lastDiagnostics,
    attempt,
    reason,
    lastConnectionStartedAt: connectingSince,
  };
  nextConnectReason = "topic-change";
  setConnectionPhase(hasConnectedOnce ? "reconnecting" : "connecting");
  activeConnectionSignature = signature;
  emitDiagnostics({
    attempt,
    reason,
    activeTopics: request.activeTopics,
    resumeTopics: request.resumeTopics,
    forcedSnapshotTopics: request.forcedSnapshotTopics,
    lastConnectionStartedAt: connectingSince,
  });
  eventSource = createEventSource(request.path);
  eventSource.addEventListener("message", handleMessage as EventListener);
  eventSource.addEventListener("error", handleError);
  eventSource.addEventListener("open", handleOpen);
  startConnectionWatchdog();
  return eventSource;
}

function cleanupEventSource() {
  if (hasActiveTopicSubscribers()) {
    return;
  }
  destroyEventSource();
  stopConnectionWatchdog();
  clearReconnectTimer();
  reconnectAttempts = 0;
  hasConnectedOnce = false;
  sseDisabled = false;
  nextConnectReason = "initial";
  resetDowntimeWindow();
  emitDiagnostics({
    activeTopics: [],
    resumeTopics: [],
    forcedSnapshotTopics: [],
    lastTerminalOutcome: "cleanup",
  });
  setConnectionPhase("idle");
}

function scheduleReconnect(options: { immediate?: boolean; reason?: SseReconnectReason } = {}) {
  if (!hasActiveTopicSubscribers()) return;
  if (sseDisabled) return;
  const { immediate = false, reason } = options;
  if (!immediate && reconnectTimer != null) return;

  if (reason) {
    nextConnectReason = reason;
    emitDiagnostics({ reason });
  }
  clearReconnectTimer();
  destroyEventSource();

  const delay = immediate
    ? 0
    : Math.min(BASE_RECONNECT_DELAY_MS * 2 ** reconnectAttempts, MAX_RECONNECT_DELAY_MS);
  const nextAttempts = Math.min(reconnectAttempts + 1, 50);

  nextRetryAt = Date.now() + delay;
  emitStatus();
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    reconnectAttempts = nextAttempts;
    nextRetryAt = null;
    ensureEventSource();
    emitStatus();
  }, delay);
  setConnectionPhase(hasConnectedOnce ? "reconnecting" : "connecting");
}

function startConnectionWatchdog() {
  if (connectionWatchdog != null) return;
  connectionWatchdog = window.setInterval(() => {
    if (sseDisabled || !eventSource) return;
    if (eventSource.readyState === EventSource.OPEN) {
      connectingSince = null;
      return;
    }
    if (eventSource.readyState === EventSource.CLOSED) {
      beginDowntimeWindow();
      emitDiagnostics({ lastTerminalOutcome: "watchdog-closed" });
      scheduleReconnect({ reason: "watchdog-closed" });
      return;
    }
    if (eventSource.readyState === EventSource.CONNECTING) {
      if (connectingSince == null) {
        connectingSince = Date.now();
      }
      if (connectingSince != null && Date.now() - connectingSince > CONNECTING_TIMEOUT_MS) {
        beginDowntimeWindow();
        emitDiagnostics({ lastTerminalOutcome: "watchdog-timeout" });
        scheduleReconnect({ reason: "watchdog-timeout" });
      }
    }
  }, WATCHDOG_INTERVAL_MS);
}

function rebuildConnection(reason: SseReconnectReason) {
  if (!hasActiveTopicSubscribers()) {
    cleanupEventSource();
    return;
  }
  if (eventSource) {
    beginDowntimeWindow();
  }
  reconnectAttempts = 0;
  nextConnectReason = reason;
  scheduleReconnect({ immediate: true, reason });
}

function markAllActiveTopicsForFreshSnapshot() {
  for (const descriptorKey of topicEntries.keys()) {
    forcedSnapshotDescriptors.add(descriptorKey);
  }
  emitDiagnostics();
}

export function subscribeToTopic<T = unknown>(
  descriptor: SubscriptionTopicDescriptor,
  listener: TopicListener<T>,
) {
  const normalized = normalizeIncomingDescriptor(descriptor);
  const key = getTopicDescriptorKey(normalized);
  const hadSubscribers = hasActiveTopicSubscribers();
  const existing = topicEntries.get(key);
  if (existing) {
    existing.listeners.add(listener as TopicListener);
  } else {
    topicEntries.set(key, {
      descriptor: normalized,
      listeners: new Set([listener as TopicListener]),
    });
    if (hadSubscribers) {
      nextConnectReason = "topic-change";
    }
  }
  const cached = topicCache.get(key);
  if (cached?.payload != null && cached.topicKey && cached.cursor != null && cached.schemaEpoch) {
    listener({
      type: cached.lastKind ?? "snapshot",
      topic: normalized,
      topicKey: cached.topicKey,
      schemaEpoch: cached.schemaEpoch,
      cursor: cached.cursor,
      payload: cached.payload as T,
    });
  }
  emitDiagnostics();
  ensureEventSource();
  return () => {
    const entry = topicEntries.get(key);
    if (!entry) return;
    entry.listeners.delete(listener as TopicListener);
    if (entry.listeners.size === 0) {
      topicEntries.delete(key);
      forcedSnapshotDescriptors.delete(key);
    }
    emitDiagnostics();
    cleanupEventSource();
  };
}

export function getCachedTopicState<T = unknown>(
  descriptor: SubscriptionTopicDescriptor,
): SubscriptionTopicState<T> | null {
  const normalized = normalizeIncomingDescriptor(descriptor);
  const cached = topicCache.get(getTopicDescriptorKey(normalized));
  if (!cached) return null;
  return cached as SubscriptionTopicState<T>;
}

export function requestTopicRefresh(descriptor: SubscriptionTopicDescriptor) {
  const key = getTopicDescriptorKey(normalizeIncomingDescriptor(descriptor));
  forcedSnapshotDescriptors.add(key);
  emitDiagnostics();
  if (hasActiveTopicSubscribers()) {
    rebuildConnection("topic-refresh");
  }
}

export function subscribeToSseOpen(listener: OpenListener) {
  openListeners.add(listener);
  return () => {
    openListeners.delete(listener);
  };
}

export function subscribeToSseActivity(listener: ActivityListener) {
  activityListeners.add(listener);
  return () => {
    activityListeners.delete(listener);
  };
}

export function subscribeToSseStatus(listener: StatusListener) {
  statusListeners.add(listener);
  listener(lastStatus);
  return () => {
    statusListeners.delete(listener);
    cleanupEventSource();
  };
}

export function subscribeToSseDiagnostics(listener: DiagnosticsListener) {
  diagnosticsListeners.add(listener);
  listener(lastDiagnostics);
  return () => {
    diagnosticsListeners.delete(listener);
    cleanupEventSource();
  };
}

export function getCurrentSseStatus() {
  return lastStatus;
}

export function getCurrentSseDiagnostics() {
  return lastDiagnostics;
}

export function requestImmediateReconnect() {
  if (!hasActiveTopicSubscribers()) return;
  if (!isEventSourceSupported()) return;
  if (sseDisabled) {
    sseDisabled = false;
  }
  markAllActiveTopicsForFreshSnapshot();
  beginDowntimeWindow();
  reconnectAttempts = 0;
  rebuildConnection("manual");
}

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState !== "visible") return;
    const status = getCurrentSseStatus();
    if (status.phase === "connected" || status.phase === "idle") return;
    nextConnectReason = "visibility-visible";
    emitDiagnostics({ reason: "visibility-visible" });
    if (sseDisabled) {
      sseDisabled = false;
    }
    reconnectAttempts = 0;
    beginDowntimeWindow();
    rebuildConnection("visibility-visible");
  });
}

if (typeof window !== "undefined" && import.meta.env.DEV) {
  window.__CVM_SSE__ = {
    requestImmediateReconnect,
    getCurrentSseDiagnostics,
    getCurrentSseStatus,
  };
}
