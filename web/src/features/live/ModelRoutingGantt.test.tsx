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
    constructor(
      host: HTMLElement,
      tasks: Array<{ id: string; name: string }>,
      options: { column_width: number },
    ) {
      ganttMocks.construct();
      const container = document.createElement("div");
      container.className = "gantt-container";
      const upperHeader = document.createElement("div");
      upperHeader.className = "upper-header";
      const lowerHeader = document.createElement("div");
      lowerHeader.className = "lower-header";
      for (let index = 1; index <= 5; index += 1) {
        const lowerText = document.createElement("div");
        lowerText.className = "lower-text";
        lowerText.style.left = `${index * options.column_width}px`;
        lowerHeader.appendChild(lowerText);
        const upperText = document.createElement("div");
        upperText.className = "upper-text";
        upperText.style.left = `${index * options.column_width}px`;
        upperHeader.appendChild(upperText);
      }
      const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
      svg.setAttribute("class", "gantt");
      tasks.forEach((task, index) => {
        const wrapper = document.createElementNS("http://www.w3.org/2000/svg", "g");
        wrapper.setAttribute("class", "bar-wrapper");
        wrapper.setAttribute("data-id", task.id);
        const barGroup = document.createElementNS("http://www.w3.org/2000/svg", "g");
        barGroup.setAttribute("class", "bar-group");
        const bar = document.createElementNS("http://www.w3.org/2000/svg", "rect");
        bar.setAttribute("class", "bar");
        bar.setAttribute("x", String(options.column_width));
        bar.setAttribute("y", String(54 + index * 32));
        bar.setAttribute("width", String(options.column_width * 4));
        bar.setAttribute("height", "22");
        const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
        label.setAttribute("class", "bar-label");
        label.textContent = task.name;
        barGroup.append(bar, label);
        wrapper.appendChild(barGroup);
        svg.appendChild(wrapper);
      });
      container.append(upperHeader, lowerHeader, svg);
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
  Object.defineProperty(SVGElement.prototype, "getComputedTextLength", {
    configurable: true,
    writable: true,
    value: function getComputedTextLength(this: SVGElement) {
      return (this.textContent?.length ?? 0) * 8;
    },
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

  it("updates the existing time axis when a live snapshot advances", () => {
    render(firstSnapshot);
    const lowerText = host?.querySelector<HTMLElement>(".lower-text");
    if (!lowerText) throw new Error("Gantt lower axis is missing");
    const initialLabel = lowerText.textContent;

    render({ ...firstSnapshot, generatedAt: "2026-08-24T02:01:00Z" });

    expect(ganttMocks.construct).toHaveBeenCalledTimes(1);
    expect(lowerText.textContent).not.toBe(initialLabel);
  });

  it("keeps a single model expansion action after a live update", () => {
    render(firstSnapshot);
    render(updatedSnapshot);
    const modelGroup = host?.querySelector<SVGGElement>(
      '[data-testid="model-routing-model-group-gpt-5.4"]',
    );
    if (!modelGroup) throw new Error("Model group is missing");

    act(() => {
      modelGroup.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(
      host
        ?.querySelector('[data-testid="model-routing-model-group-gpt-5.4"]')
        ?.getAttribute("aria-expanded"),
    ).toBe("true");
  });

  it("reconstructs the Gantt layout when a lane display name changes", () => {
    render(firstSnapshot);
    render({
      ...firstSnapshot,
      groups: [
        {
          ...firstSnapshot.groups[0],
          accounts: [
            {
              ...firstSnapshot.groups[0].accounts[0],
              accountDisplayName: "Borealis",
            },
          ],
        },
      ],
    });

    expect(ganttMocks.construct).toHaveBeenCalledTimes(2);
  });

  it("restores a truncated lane name and title after a live update", () => {
    const displayName = "Northstar Production Gateway for Long Context Requests";
    const snapshot = {
      ...firstSnapshot,
      groups: [
        {
          ...firstSnapshot.groups[0],
          accounts: [
            {
              ...firstSnapshot.groups[0].accounts[0],
              accountDisplayName: displayName,
            },
          ],
        },
      ],
    };
    render(snapshot);
    const lane = host?.querySelector<SVGGElement>('[data-testid="model-routing-lane-gpt-5.4-21"]');
    const label = lane?.querySelector<SVGTextElement>(".bar-label");
    expect(label?.textContent).toContain("…");
    expect(label?.querySelector("title")?.textContent).toBe(displayName);

    render({ ...snapshot, generatedAt: "2026-08-24T02:00:01Z" });

    expect(label?.textContent).toContain("…");
    expect(label?.querySelector("title")?.textContent).toBe(displayName);
  });

  it("reconstructs the Gantt layout when a newly observed model creates a group", () => {
    render(firstSnapshot);
    render({
      ...firstSnapshot,
      groups: [
        ...firstSnapshot.groups,
        {
          model: "gpt-5.5",
          accounts: [
            {
              ...firstSnapshot.groups[0].accounts[0],
              accountId: 22,
              accountDisplayName: "Cedar",
              model: "gpt-5.5",
            },
          ],
        },
      ],
    });

    expect(ganttMocks.construct).toHaveBeenCalledTimes(2);
  });
});
