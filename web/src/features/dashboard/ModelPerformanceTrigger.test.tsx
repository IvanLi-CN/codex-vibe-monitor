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
    avgFirstResponseByteTotalMs: 720,
    usageDurationMs: 90000,
  },
  models: [
    {
      model: "gpt-5.6",
      reasoningEffort: null,
      tokensPerMinute: 1200,
      streamingResponseRate: null,
      avgResponseMs: null,
      avgFirstResponseByteTotalMs: 720,
      usageDurationMs: 90000,
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
  it("opens the accessible desktop tooltip with total and unspecified effort rows", async () => {
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
    expect(tooltip?.textContent).toMatch(/Unspecified|未指定/);
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
  });

  it("normalizes rounded usage durations at the next hour boundary", async () => {
    compactViewport = true;
    await renderTrigger({
      ...modelPerformance,
      total: { ...modelPerformance.total, usageDurationMs: 7_199_500 },
      models: [{ ...modelPerformance.models[0], usageDurationMs: 7_199_500 }],
    });
    const trigger = host?.querySelector('[aria-label="Open model performance details"]');
    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(document.body.querySelector('[role="dialog"]')?.textContent).toContain("2 h");
    expect(document.body.querySelector('[role="dialog"]')?.textContent).not.toContain("1 h 60 min");
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
