/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, vi } from "vitest";
import type {
  BroadcastPayload,
  InvocationRecordsNewCountResponse,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
} from "../lib/api";
import { useInvocationRecords } from "./useInvocationRecords";

const apiMocks = vi.hoisted(() => ({
  fetchInvocationRecords:
    vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsResponse>>(),
  fetchInvocationRecordsSummary:
    vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsSummaryResponse>>(),
  fetchInvocationRecordsNewCount:
    vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsNewCountResponse>>(),
}));
const realtimeMocks = vi.hoisted(() => ({
  latest: null as null | {
    onRecordsChange: (
      next: InvocationRecordsResponse["records"],
      meta: { visibleInsertedKeys: string[]; payload: BroadcastPayload & { type: "records" } },
    ) => void;
    onOpenResync: () => void;
  },
}));
vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationRecordsSummary: apiMocks.fetchInvocationRecordsSummary,
    fetchInvocationRecordsNewCount: apiMocks.fetchInvocationRecordsNewCount,
  };
});
vi.mock("./useInvocationRecordsRealtime", () => ({
  useInvocationRecordsRealtime: (options: {
    onRecordsChange: (
      next: InvocationRecordsResponse["records"],
      meta: { visibleInsertedKeys: string[]; payload: BroadcastPayload & { type: "records" } },
    ) => void;
    onOpenResync: () => void;
  }) => {
    realtimeMocks.latest = options;
  },
}));
let host: HTMLDivElement | null = null;
let root: Root | null = null;
beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});
beforeEach(() => {
  apiMocks.fetchInvocationRecords.mockReset();
  apiMocks.fetchInvocationRecordsSummary.mockReset();
  apiMocks.fetchInvocationRecordsNewCount.mockReset();
  realtimeMocks.latest = null;
});
afterEach(() => {
  vi.useRealTimers();
});
afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  realtimeMocks.latest = null;
});
function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}
function click(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}
function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}
async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function waitFor(check: () => void, attempts = 20) {
  let lastError: unknown = null;
  for (let index = 0; index < attempts; index += 1) {
    try {
      check();
      return;
    } catch (error) {
      lastError = error;
      await flushAsync();
    }
  }
  throw lastError;
}
function invocationKey(record: InvocationRecordsResponse["records"][number]) {
  return `${record.invokeId}-${record.occurredAt}`;
}
function emitRealtimeRecords(
  next: InvocationRecordsResponse["records"],
  visibleInsertedKeys: string[] = [],
) {
  if (!realtimeMocks.latest) {
    throw new Error("Missing realtime hook registration");
  }
  act(() => {
    realtimeMocks.latest?.onRecordsChange(next, {
      visibleInsertedKeys,
      payload: {
        type: "records",
        records: next,
      } as BroadcastPayload & { type: "records" },
    });
  });
}
function createListResponse(
  overrides: Partial<InvocationRecordsResponse>,
): InvocationRecordsResponse {
  return {
    snapshotId: 42,
    total: 2,
    page: 1,
    pageSize: 20,
    records: [
      {
        id: 1,
        invokeId: "invoke-1",
        occurredAt: "2026-03-10T00:00:00Z",
        createdAt: "2026-03-10T00:00:00Z",
        model: "baseline-model",
        status: "success",
      },
    ],
    ...overrides,
  };
}
function createSummaryResponse(
  overrides: Partial<InvocationRecordsSummaryResponse>,
): InvocationRecordsSummaryResponse {
  return {
    snapshotId: 42,
    newRecordsCount: 0,
    totalCount: 2,
    successCount: 2,
    failureCount: 0,
    totalCost: 0.25,
    totalTokens: 1000,
    token: {
      requestCount: 2,
      totalTokens: 1000,
      avgTokensPerRequest: 500,
      cacheWriteTokens: 0,
      cacheInputTokens: 200,
      outputTokens: 800,
      totalCost: 0.25,
      maxTokensPerRequest: 640,
    },
    network: {
      avgTtfbMs: 120,
      p95TtfbMs: 180,
      avgTotalMs: 450,
      p95TotalMs: 600,
      maxTotalMs: 800,
    },
    exception: {
      failureCount: 0,
      serviceFailureCount: 0,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: 0,
    },
    ...overrides,
  };
}
function createNewCountResponse(
  overrides: Partial<InvocationRecordsNewCountResponse>,
): InvocationRecordsNewCountResponse {
  return {
    snapshotId: 42,
    newRecordsCount: 0,
    ...overrides,
  };
}
function createRecord(overrides: Partial<InvocationRecordsResponse["records"][number]> = {}) {
  return {
    id: 1,
    invokeId: "invoke-1",
    occurredAt: "2026-03-10T00:00:00Z",
    createdAt: "2026-03-10T00:00:00Z",
    model: "baseline-model",
    status: "success",
    ...overrides,
  };
}
function Probe() {
  const state = useInvocationRecords();
  return (
    <div>
      <div data-testid="focus">{state.focus}</div>
      <div data-testid="page">{state.page}</div>
      <div data-testid="page-size">{state.pageSize}</div>
      <div data-testid="snapshot">{state.records?.snapshotId ?? 0}</div>
      <div data-testid="total">{state.records?.total ?? 0}</div>
      <div data-testid="model">{state.records?.records[0]?.model ?? ""}</div>
      <div data-testid="account-name">{state.records?.records[0]?.upstreamAccountName ?? ""}</div>
      <div data-testid="new-count">{state.summary?.newRecordsCount ?? 0}</div>
      <div data-testid="summary-snapshot">{state.summary?.snapshotId ?? 0}</div>
      <div data-testid="records-loading">{state.isRecordsLoading ? "yes" : "no"}</div>
      <div data-testid="summary-loading">{state.isSummaryLoading ? "yes" : "no"}</div>
      <div data-testid="records-error">{state.recordsError ?? ""}</div>
      <div data-testid="summary-error">{state.summaryError ?? ""}</div>
      <div data-testid="applied-draft-model">{state.appliedDraft?.model ?? ""}</div>
      <button data-testid="focus-network" type="button" onClick={() => state.setFocus("network")}>
        network
      </button>
      <button
        data-testid="draft-model"
        type="button"
        onClick={() => state.updateDraft("model", "next-model")}
      >
        draft
      </button>
      <button
        data-testid="failed-status"
        type="button"
        onClick={() => state.updateDraft("status", "failed")}
      >
        failed
      </button>
      <div data-testid="applied-model">
        {state.records?.records.map((record) => record.model ?? "").join("|")}
      </div>
      <button data-testid="page-2" type="button" onClick={() => void state.setPage(2)}>
        page2
      </button>
      <button data-testid="page-size-50" type="button" onClick={() => void state.setPageSize(50)}>
        pageSize50
      </button>
      <button data-testid="search" type="button" onClick={() => void state.search()}>
        search
      </button>
      <button
        data-testid="refresh-applied"
        type="button"
        onClick={() => void state.search({ source: "applied", preserveSummary: true })}
      >
        refreshApplied
      </button>
    </div>
  );
}

export {
  apiMocks,
  click,
  createListResponse,
  createNewCountResponse,
  createRecord,
  createSummaryResponse,
  emitRealtimeRecords,
  flushAsync,
  host,
  invocationKey,
  Probe,
  realtimeMocks,
  render,
  root,
  text,
  waitFor,
};
