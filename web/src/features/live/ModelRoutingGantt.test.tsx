/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingLiveResponse } from "../../lib/api";
import { ThemeProvider } from "../../theme";

const ganttMocks = vi.hoisted(() => ({
  construct: vi.fn(),
}));

vi.mock("frappe-gantt", () => ({
  default: class MockGantt {
    constructor(host: HTMLElement) {
      ganttMocks.construct();
      const container = document.createElement("div");
      container.className = "gantt-container";
      host.appendChild(container);
    }
  },
}));

import { ModelRoutingGantt } from "./ModelRoutingGantt";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

const firstSnapshot: ModelRoutingLiveResponse = {
  generatedAt: "2026-08-24T02:00:00Z",
  groups: [
    {
      model: "gpt-5.4",
      accounts: [
        {
          accountId: 21,
          accountDisplayName: "Aster",
          model: "gpt-5.4",
          state: "available",
          priority: "normal",
          failureCount: 0,
          lastSeenAt: "2026-08-24T02:00:00Z",
        },
      ],
    },
  ],
  records: [],
};

const updatedSnapshot: ModelRoutingLiveResponse = {
  ...firstSnapshot,
  generatedAt: "2026-08-24T02:00:01Z",
  records: [
    {
      id: "attempt:1",
      kind: "attempt",
      occurredAt: "2026-08-24T01:59:59Z",
      accountId: 21,
      accountDisplayName: "Aster",
      model: "gpt-5.4",
      attemptId: "attempt-public-1",
      status: "success",
      httpStatus: 200,
    },
  ],
};

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get: () => 1200,
  });
  Object.defineProperty(globalThis, "requestAnimationFrame", {
    configurable: true,
    writable: true,
    value: (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    },
  });
  Object.defineProperty(globalThis, "cancelAnimationFrame", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  ganttMocks.construct.mockReset();
});

function render(snapshot: ModelRoutingLiveResponse) {
  if (!host) {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  }
  act(() => {
    root?.render(
      <I18nProvider>
        <ThemeProvider>
          <ModelRoutingGantt
            groups={snapshot.groups}
            records={snapshot.records}
            generatedAt={snapshot.generatedAt}
            window="1h"
            onOpenAccount={vi.fn()}
            onOpenInvocation={vi.fn()}
          />
        </ThemeProvider>
      </I18nProvider>,
    );
  });
}

describe("ModelRoutingGantt", () => {
  it("does not reconstruct the Gantt layout or reset horizontal scroll when a live record updates existing lanes", () => {
    render(firstSnapshot);
    expect(ganttMocks.construct).toHaveBeenCalledTimes(1);
    const ganttContainer = host?.querySelector<HTMLElement>(".gantt-container");
    if (!ganttContainer) throw new Error("Gantt container is missing");
    ganttContainer.scrollLeft = 180;

    render(updatedSnapshot);

    expect(ganttMocks.construct).toHaveBeenCalledTimes(1);
    expect(ganttContainer.scrollLeft).toBe(180);
  });

  it("reconstructs the Gantt layout when a newly observed account creates a lane", () => {
    render(firstSnapshot);
    expect(ganttMocks.construct).toHaveBeenCalledTimes(1);

    render({
      ...firstSnapshot,
      records: [
        {
          id: "attempt:new-lane",
          kind: "attempt",
          occurredAt: "2026-08-24T01:59:59Z",
          accountId: 22,
          accountDisplayName: "Borealis",
          model: "gpt-5.4",
          attemptId: "attempt-public-new-lane",
          status: "success",
          httpStatus: 200,
        },
      ],
    });

    expect(ganttMocks.construct).toHaveBeenCalledTimes(2);
  });
});
