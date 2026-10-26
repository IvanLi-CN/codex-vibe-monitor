/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { StorageMigrationResponse } from "../../lib/api";
import SystemStorageMigrationPage from "./SystemStorageMigrationPage";

const apiMocks = vi.hoisted(() => ({
  createStorageMigrationEventSource: vi.fn(),
  fetchStorageMigrationStatus: vi.fn(),
  updateStorageMigrationControl: vi.fn(),
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    createStorageMigrationEventSource: apiMocks.createStorageMigrationEventSource,
    fetchStorageMigrationStatus: apiMocks.fetchStorageMigrationStatus,
    updateStorageMigrationControl: apiMocks.updateStorageMigrationControl,
  };
});

class FakeEventSource {
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  private readonly listeners = new Map<string, Array<(event: Event) => void>>();

  addEventListener(type: string, listener: (event: Event) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }

  close() {
    this.closed = true;
  }

  emit(type: string, data: string) {
    const event = new MessageEvent(type, { data });
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

const response: StorageMigrationResponse = {
  revision: 7,
  migrationId: "storage-migration",
  active: true,
  module: "traffic_archive",
  state: "copying",
  source: { backend: "sqlite", configured: true, identity: "sqlite:local" },
  target: { backend: "postgres", configured: true, identity: "postgres:configured" },
  authority: { backend: "sqlite", generation: 1 },
  queuedModules: ["analytics"],
  completedModules: [],
  heartbeat: { at: "2026-10-26T08:01:12Z", healthy: true },
  progress: {
    committedRows: 820,
    committedBytes: 1_900_000,
    sourceRows: 1_000,
    sourceBytes: 2_400_000,
    scanCursor: "archive_events:820",
    capturedThrough: 890,
    appliedThrough: 887,
    pendingKeys: 34,
    oldestChangeAgeMs: 11_000,
    replayThroughputPerSec: 21,
    incomingRatePerSec: 15,
    converged: true,
    etaMs: 90_000,
    phase: "copying",
  },
  tables: [],
  backlog: { sourceP1: 34, pendingKeys: 34, oldestChangeAgeMs: 11_000 },
  verification: { dirtyChunks: null, verifiedChunks: null, lastError: null },
  taskHolds: {
    held: false,
    preservedTaskKeys: [],
    reason: null,
    drainState: "not_started",
    lastSafeCheckpoint: "archive_events:820",
  },
  pressure: {
    sourceBacklog: 34,
    lockWaitMs: 12,
    copyThroughputPerSec: 21,
    incomingChangeRatePerSec: 15,
    diskHeadroomBytes: 18_000_000_000,
    walHeadroomBytes: null,
    freshnessAgeMs: 1_500,
    nonConvergent: false,
  },
  switch: {
    budgetMs: 3_000,
    attemptCount: 0,
    lastAttemptAt: null,
    durationMs: null,
    result: null,
    failureReason: null,
  },
  modules: [],
  recovery: {
    required: false,
    reason: null,
    authorityBackend: "sqlite",
    authorityGeneration: 1,
  },
  controls: {
    permission: "admin",
    canStart: false,
    canPause: true,
    canResume: false,
    lastAction: null,
    lastActionAt: null,
  },
  updatedAt: "2026-10-26T08:01:12Z",
};

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let eventSource: FakeEventSource | null = null;
let originalEventSource: typeof window.EventSource | undefined;

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider initialLocale="zh">
        <SystemStorageMigrationPage />
      </I18nProvider>,
    );
  });
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("SystemStorageMigrationPage", () => {
  beforeEach(() => {
    originalEventSource = window.EventSource;
    Object.defineProperty(window, "EventSource", {
      configurable: true,
      value: FakeEventSource,
    });
    eventSource = new FakeEventSource();
    apiMocks.fetchStorageMigrationStatus.mockResolvedValue(response);
    apiMocks.createStorageMigrationEventSource.mockImplementation(
      () => eventSource as unknown as EventSource,
    );
    apiMocks.updateStorageMigrationControl.mockResolvedValue({
      ...response,
      state: "paused",
      taskHolds: {
        held: true,
        preservedTaskKeys: [],
        reason: "operator_pause",
        drainState: "draining",
        lastSafeCheckpoint: "archive_events:820",
      },
      controls: { ...response.controls, canPause: false, canResume: true, lastAction: "pause" },
    });
  });

  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    root = null;
    host = null;
    eventSource = null;
    if (originalEventSource) {
      Object.defineProperty(window, "EventSource", {
        configurable: true,
        value: originalEventSource,
      });
    } else {
      delete (window as Partial<Window>).EventSource;
    }
    apiMocks.createStorageMigrationEventSource.mockReset();
    apiMocks.fetchStorageMigrationStatus.mockReset();
    apiMocks.updateStorageMigrationControl.mockReset();
  });

  it("shows the worker status and keeps a live event source", async () => {
    renderPage();
    await flushEffects();

    expect(host?.textContent ?? "").toContain("存储迁移");
    expect(host?.textContent ?? "").toContain("复制中");
    expect(host?.textContent ?? "").toContain("820");
    expect(apiMocks.createStorageMigrationEventSource).toHaveBeenCalledTimes(1);
  });

  it("applies an SSE status update without a full page refresh", async () => {
    renderPage();
    await flushEffects();

    act(() => {
      eventSource?.emit(
        "status",
        JSON.stringify({
          ...response,
          state: "paused",
          progress: { ...response.progress, phase: "paused" },
          taskHolds: { held: true, preservedTaskKeys: [], reason: "operator_pause" },
          controls: { ...response.controls, canPause: false, canResume: true },
        }),
      );
    });

    expect(host?.textContent ?? "").toContain("已暂停");
    expect(apiMocks.fetchStorageMigrationStatus).toHaveBeenCalledTimes(1);
  });

  it("sends an admin pause action through the control endpoint", async () => {
    renderPage();
    await flushEffects();

    const pauseButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) =>
      button.textContent?.includes("暂停"),
    );
    expect(pauseButton).toBeTruthy();
    await act(async () => {
      pauseButton?.click();
      await Promise.resolve();
    });
    expect(apiMocks.updateStorageMigrationControl).toHaveBeenCalledWith(true);
  });
});
