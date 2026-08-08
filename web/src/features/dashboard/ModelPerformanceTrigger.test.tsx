/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelPerformance } from "../../lib/api";
import { ModelPerformanceTrigger } from "./ModelPerformanceTrigger";

const modelPerformance: ModelPerformance = {
  available: true,
  total: {
    tokensPerMinute: 1200,
    streamingResponseRate: 150,
    avgResponseMs: 2800,
    avgFirstTokenMs: 720,
    wallClockUsageDurationMs: 90000,
    cumulativeUsageDurationMs: 288000,
    parallelism: 3.2,
  },
  models: [
    {
      model: "gpt-5.6-sol",
      reasoningEffort: " MAX ",
      tokensPerMinute: 1200,
      streamingResponseRate: null,
      avgResponseMs: null,
      avgFirstTokenMs: 720,
      wallClockUsageDurationMs: 90000,
      cumulativeUsageDurationMs: 288000,
      parallelism: 3.2,
    },
    {
      model: "gpt-5.6-terra-experimental-routing-variant-with-a-very-long-name",
      reasoningEffort: "adaptive-experimental",
      tokensPerMinute: 480,
      streamingResponseRate: 72,
      avgResponseMs: 4100,
      avgFirstTokenMs: 830,
      wallClockUsageDurationMs: 64000,
      cumulativeUsageDurationMs: 72000,
      parallelism: 1.13,
    },
  ],
};

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let compactViewport = false;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn(() => ({
      matches: compactViewport,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  compactViewport = false;
});

async function renderTrigger(performance = modelPerformance) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  await act(async () => {
    root?.render(
      <I18nProvider>
        <ModelPerformanceTrigger
          title="Model performance"
          ariaLabel="Open model performance details"
          performance={performance}
        >
          <span>Open details</span>
        </ModelPerformanceTrigger>
      </I18nProvider>,
    );
    await Promise.resolve();
  });
}

describe("ModelPerformanceTrigger", () => {
  it("opens the accessible desktop tooltip with single-line model identity badges", async () => {
    await renderTrigger();
    const trigger = host?.querySelector('[aria-label="Open model performance details"]');
    expect(trigger).toBeInstanceOf(HTMLElement);

    await act(async () => {
      trigger?.dispatchEvent(new FocusEvent("focus", { bubbles: true }));
      trigger?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await Promise.resolve();
    });

    const tooltip = document.body.querySelector('[role="tooltip"]');
    expect(tooltip?.textContent).toContain("Model performance");
    expect(tooltip?.textContent).toMatch(/Total|总计/);
    const modelContexts = tooltip?.querySelectorAll(
      '[data-testid="model-performance-table-model-context"]',
    );
    expect(modelContexts).toHaveLength(2);
    expect(modelContexts?.[0]?.getAttribute("data-model-context-display")).toBe("model-badge");
    expect(modelContexts?.[0]?.getAttribute("title")).toContain("gpt-5.6-sol");
    expect(modelContexts?.[0]?.querySelector('[data-testid$="-name"]')).toBeNull();
    expect(modelContexts?.[0]?.querySelector('[data-reasoning-effort-tone="max"]')).not.toBeNull();
    expect(modelContexts?.[0]?.textContent).toContain("max");
    expect(modelContexts?.[1]?.getAttribute("data-model-context-display")).toBe("name-and-effort");
    expect(modelContexts?.[1]?.querySelector('[data-testid$="-name"]')?.getAttribute("title")).toBe(
      "gpt-5.6-terra-experimental-routing-variant-with-a-very-long-name",
    );
    expect(
      modelContexts?.[1]?.querySelector('[data-reasoning-effort-tone="unknown"]'),
    ).not.toBeNull();
    expect(tooltip?.textContent).toMatch(/Wall clock|墙钟时长/);
    expect(tooltip?.textContent).toMatch(/Cumulative|累计时长/);
    expect(tooltip?.textContent).toMatch(/Parallelism|并行数/);
    expect(tooltip?.textContent).toContain("x3.20");
  });

  it("opens a compact drawer without a horizontally scrolling table", async () => {
    compactViewport = true;
    await renderTrigger();
    const trigger = host?.querySelector('[aria-label="Open model performance details"]');
    expect(trigger?.tagName).toBe("BUTTON");

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog?.textContent).toContain("Model performance");
    expect(dialog?.querySelector("table")).toBeNull();
    expect(
      dialog?.querySelector('[data-testid="model-performance-drawer-content"]'),
    ).not.toBeNull();
    expect(dialog?.textContent).toMatch(/Wall clock|墙钟时长/);
    expect(dialog?.textContent).toContain("x3.20");
    expect(
      dialog?.querySelector('[data-testid="model-performance-drawer-model-context"]'),
    ).not.toBeNull();
  });

  it("normalizes rounded wall-clock durations and fixed parallelism formatting", async () => {
    compactViewport = true;
    await renderTrigger({
      ...modelPerformance,
      total: {
        ...modelPerformance.total,
        wallClockUsageDurationMs: 7_199_500,
        cumulativeUsageDurationMs: 23_038_400,
        parallelism: 3.2,
      },
      models: [
        {
          ...modelPerformance.models[0],
          wallClockUsageDurationMs: 7_199_500,
          cumulativeUsageDurationMs: 23_038_400,
          parallelism: 3.2,
        },
      ],
    });
    const trigger = host?.querySelector('[aria-label="Open model performance details"]');
    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(document.body.querySelector('[role="dialog"]')?.textContent).toContain("2 h");
    expect(document.body.querySelector('[role="dialog"]')?.textContent).not.toContain("1 h 60 min");
    expect(document.body.querySelector('[role="dialog"]')?.textContent).toContain("x3.20");
  });

  it("renders explicit empty and unavailable states", async () => {
    await renderTrigger({ available: true, total: { tokensPerMinute: 0 }, models: [] });
    const trigger = host?.querySelector('[aria-label="Open model performance details"]');
    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(document.body.querySelector('[data-testid="model-performance-empty"]')).not.toBeNull();

    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;

    await renderTrigger({ available: false, total: { tokensPerMinute: 0 }, models: [] });
    const unavailableTrigger = host?.querySelector('[aria-label="Open model performance details"]');
    await act(async () => {
      unavailableTrigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="model-performance-unavailable"]'),
    ).not.toBeNull();
  });
});
