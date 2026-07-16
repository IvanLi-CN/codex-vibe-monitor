/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiInvocation, ListResponse } from "../lib/api";
import { useInvocationStream } from "./useInvocations";

const topicMocks = vi.hoisted(() => ({
  state: {
    data: null as ListResponse | null,
    isLoading: false,
    error: null as string | null,
    refresh: vi.fn(),
  },
  lastDescriptor: null as Record<string, unknown> | null,
  lastEnabled: true,
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: Record<string, unknown> | null, enabled = true) => {
    topicMocks.lastDescriptor = descriptor;
    topicMocks.lastEnabled = enabled;
    return topicMocks.state;
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
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
  topicMocks.state.refresh.mockReset();
  topicMocks.lastDescriptor = null;
  topicMocks.lastEnabled = true;
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.clearAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function createRecord(overrides: Partial<ApiInvocation> = {}): ApiInvocation {
  return {
    id: overrides.id ?? 1,
    invokeId: overrides.invokeId ?? "invoke-1",
    occurredAt: overrides.occurredAt ?? "2026-07-16T10:00:00Z",
    createdAt: overrides.createdAt ?? overrides.occurredAt ?? "2026-07-16T10:00:00Z",
    status: overrides.status ?? "success",
    ...overrides,
  };
}

function Probe({
  limit = 20,
  enableStream = true,
  onNewRecords,
}: {
  limit?: number;
  enableStream?: boolean;
  onNewRecords?: (records: ApiInvocation[]) => void;
}) {
  const { records, hasData, isLoading } = useInvocationStream(
    limit,
    { model: "gpt-5.4", status: "failed" },
    onNewRecords,
    { enableStream },
  );

  return (
    <div>
      <div data-testid="count">{String(records.length)}</div>
      <div data-testid="first-id">{records[0]?.invokeId ?? ""}</div>
      <div data-testid="has-data">{hasData ? "true" : "false"}</div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
    </div>
  );
}

describe("useInvocationStream", () => {
  it("subscribes to the invocations.window topic and exposes authoritative records", () => {
    topicMocks.state.data = {
      records: [
        createRecord({ invokeId: "invoke-a" }),
        createRecord({ invokeId: "invoke-b", id: 2 }),
      ],
    } as ListResponse;

    render(<Probe />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "invocations.window",
      params: {
        limit: "20",
        model: "gpt-5.4",
        status: "failed",
      },
    });
    expect(topicMocks.lastEnabled).toBe(true);
    expect(text("count")).toBe("2");
    expect(text("first-id")).toBe("invoke-a");
    expect(text("has-data")).toBe("true");
  });

  it("fires onNewRecords only when the authoritative topic payload changes", () => {
    const onNewRecords = vi.fn();
    topicMocks.state.data = {
      records: [createRecord({ invokeId: "invoke-a" })],
    } as ListResponse;

    render(<Probe onNewRecords={onNewRecords} />);
    expect(onNewRecords).toHaveBeenCalledTimes(1);

    rerender(<Probe onNewRecords={onNewRecords} />);
    expect(onNewRecords).toHaveBeenCalledTimes(1);

    topicMocks.state.data = {
      records: [createRecord({ invokeId: "invoke-b", id: 2 })],
    } as ListResponse;
    rerender(<Probe onNewRecords={onNewRecords} />);
    expect(onNewRecords).toHaveBeenCalledTimes(2);
    expect(onNewRecords).toHaveBeenLastCalledWith([
      expect.objectContaining({ invokeId: "invoke-b" }),
    ]);
  });

  it("disables the topic subscription when stream is turned off", () => {
    render(<Probe enableStream={false} />);

    expect(topicMocks.lastDescriptor).toBeNull();
    expect(topicMocks.lastEnabled).toBe(false);
    expect(text("count")).toBe("0");
  });
});
