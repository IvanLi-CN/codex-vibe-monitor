/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { I18nProvider } from "../../i18n";
import type { UsageBreakdown } from "../../lib/api";
import {
  DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY,
  DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX,
} from "./dashboardModelBreakdown";
import { UsageBreakdownTooltip } from "./UsageBreakdownTooltip";

const labels = {
  total: "Total",
  model: "Model",
  cacheWrite: "Cache write",
  cacheRead: "Cache read",
  cacheHitRate: "Cache hit rate",
  output: "Output",
  unknownModel: "Unidentified model",
  reasoningEffort: "Reasoning effort",
};

function exactBreakdown(): UsageBreakdown {
  return {
    cacheWriteTokens: 100,
    cacheReadTokens: 20,
    outputTokens: 30,
    costs: {
      input: 1,
      cacheWrite: 2,
      cacheRead: 0.5,
      output: 1,
      reasoning: 0.5,
      unknown: 0,
    },
    models: [],
  };
}

function renderTooltip(breakdown: UsageBreakdown) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => {
    root.render(
      <I18nProvider>
        <UsageBreakdownTooltip
          title="Usage details"
          breakdown={breakdown}
          formatNumber={(value) => `T${value}`}
          formatRatio={(value) => (value == null ? "—" : `${(value * 100).toFixed(1)}%`)}
          formatCurrency={(value) => `$${value.toFixed(2)}`}
          labels={labels}
        />
      </I18nProvider>,
    );
  });
  return { host, root };
}

function totalRowCells(host: HTMLElement) {
  const row = host.querySelector("tbody tr");
  if (!row) throw new Error("missing total row");
  return Array.from(row.querySelectorAll("td")).map((cell) => cell.textContent);
}

afterEach(() => {
  document.body.replaceChildren();
  window.localStorage.removeItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY);
  window.localStorage.removeItem(
    `${DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX}.usage-breakdown`,
  );
});

beforeEach(() => {
  window.localStorage.removeItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY);
  window.localStorage.removeItem(
    `${DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX}.usage-breakdown`,
  );
});

describe("UsageBreakdownTooltip", () => {
  it("pairs cache and output Token buckets with their reconciled cost totals", () => {
    const { host, root } = renderTooltip(exactBreakdown());

    expect(
      Array.from(host.querySelectorAll("thead th")).map((header) => header.textContent),
    ).toEqual(["Model", "Cache write", "Cache read", "Cache hit rate", "Output", "Total"]);
    expect(host.querySelector("select")).toBeNull();
    expect(
      host.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-write"]'),
    ).toBeNull();
    expect(
      host.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-read"]'),
    ).toBeNull();
    expect(host.querySelector('[data-testid="dashboard-model-breakdown-sort-output"]')).toBeNull();
    expect(
      host.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-hit-rate"]'),
    ).not.toBeNull();
    expect(
      host.querySelector('[data-testid="dashboard-model-breakdown-sort-total"]'),
    ).not.toBeNull();
    expect(host.querySelector('[data-testid="usage-breakdown-mobile-list"]')).not.toBeNull();
    expect(host.querySelector('[data-testid="usage-breakdown-mobile-controls"]')).not.toBeNull();
    expect(totalRowCells(host)).toEqual([
      "T100$3.00",
      "T20$0.50",
      "13.3%",
      "T30$1.50",
      "T150$5.00",
    ]);
    const cacheHitRateCell = host.querySelector("tbody tr td:nth-of-type(3)");
    expect(cacheHitRateCell?.classList.contains("font-normal")).toBe(true);
    const cacheHitRateValue = cacheHitRateCell?.querySelector("span span:not([aria-hidden])");
    expect(cacheHitRateValue?.classList.contains("text-base-content")).toBe(true);
    expect(cacheHitRateValue?.classList.contains("text-base-content/80")).toBe(false);
    const placeholder = cacheHitRateCell?.querySelector('[aria-hidden="true"]');
    expect(placeholder?.classList.contains("h-3")).toBe(true);
    expect(placeholder?.classList.contains("sm:h-4")).toBe(true);

    act(() => root.unmount());
  });

  it("keeps historical unknown cost in total while leaving unmappable amount cells blank", () => {
    const breakdown = exactBreakdown();
    breakdown.costs = {
      input: 0,
      cacheWrite: 0,
      cacheRead: 0,
      output: 0,
      reasoning: 0,
      unknown: 5,
    };
    const { host, root } = renderTooltip(breakdown);

    expect(totalRowCells(host)).toEqual(["T100—", "T20—", "13.3%", "T30—", "T150$5.00"]);

    act(() => root.unmount());
  });

  it("shows unavailable amounts without inventing a total when cost details are absent", () => {
    const breakdown = exactBreakdown();
    delete breakdown.costs;
    const { host, root } = renderTooltip(breakdown);

    expect(totalRowCells(host)).toEqual(["T100—", "T20—", "13.3%", "T30—", "T150—"]);

    act(() => root.unmount());
  });

  it("keeps a known zero amount distinct from unavailable cost details", () => {
    const breakdown = exactBreakdown();
    breakdown.costs = {
      input: 0,
      cacheWrite: 0,
      cacheRead: 0,
      output: 0,
      reasoning: 0,
      unknown: 0,
    };
    const { host, root } = renderTooltip(breakdown);

    expect(totalRowCells(host)).toEqual([
      "T100$0.00",
      "T20$0.00",
      "13.3%",
      "T30$0.00",
      "T150$0.00",
    ]);

    act(() => root.unmount());
  });

  it("uses normalized raw reasoning efforts and the shared model identity", () => {
    const breakdown = exactBreakdown();
    breakdown.models = [
      {
        model: "gpt-5.6",
        reasoningEffort: " MAX ",
        cacheWriteTokens: 50,
        cacheReadTokens: 10,
        outputTokens: 20,
      },
      {
        model: "gpt-5.6-luna-2026-07-27",
        reasoningEffort: "ULTRA",
        cacheWriteTokens: 20,
        cacheReadTokens: 5,
        outputTokens: 5,
      },
      {
        model: "custom-model",
        reasoningEffort: null,
        cacheWriteTokens: 30,
        cacheReadTokens: 5,
        outputTokens: 5,
      },
    ];
    const { host, root } = renderTooltip(breakdown);

    expect(host.textContent).toContain("Reasoning effort: max");
    expect(host.textContent).toContain("Reasoning effort: ultra");
    expect(host.textContent).toContain("Reasoning effort: —");
    expect(host.querySelector('[data-model-identity="gpt-5.6"]')).not.toBeNull();
    expect(host.querySelector('[data-model-identity="gpt-5.6-luna-2026-07-27"]')).not.toBeNull();
    expect(host.textContent).not.toContain("MAX");
    expect(host.textContent).not.toContain("ULTRA");

    act(() => root.unmount());
  });

  it("merges effort rows in simple mode and removes the effort label", () => {
    const breakdown = exactBreakdown();
    breakdown.models = [
      {
        model: "gpt-5.6",
        reasoningEffort: "max",
        cacheWriteTokens: 50,
        cacheReadTokens: 10,
        outputTokens: 20,
      },
      {
        model: "gpt-5.6",
        reasoningEffort: "low",
        cacheWriteTokens: 20,
        cacheReadTokens: 5,
        outputTokens: 5,
      },
      {
        model: "other-model",
        reasoningEffort: "medium",
        cacheWriteTokens: 10,
        cacheReadTokens: 0,
        outputTokens: 5,
      },
    ];
    const { host, root } = renderTooltip(breakdown);
    const simpleButton = host.querySelector(
      '[data-testid="dashboard-model-breakdown-mode-simple"]',
    );

    act(() => {
      simpleButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(host.querySelectorAll("tbody tr")).toHaveLength(3);
    expect(host.textContent).not.toContain("Reasoning effort");
    expect(host.textContent).toContain("T110");
    expect(host.textContent).toContain("gpt-5.6");
    expect(window.localStorage.getItem(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY)).toBe("simple");

    act(() => root.unmount());
  });
});
