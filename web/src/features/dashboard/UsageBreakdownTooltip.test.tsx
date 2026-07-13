/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type { UsageBreakdown } from "../../lib/api";
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
  unspecifiedEffort: "Unspecified",
  effortNone: "None",
  effortMinimal: "Minimal",
  effortLow: "Low",
  effortMedium: "Medium",
  effortHigh: "High",
  effortXhigh: "XHigh",
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
      <UsageBreakdownTooltip
        title="Usage details"
        breakdown={breakdown}
        formatNumber={(value) => `T${value}`}
        formatRatio={(value) => (value == null ? "—" : `${(value * 100).toFixed(1)}%`)}
        formatCurrency={(value) => `$${value.toFixed(2)}`}
        labels={labels}
      />,
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
});

describe("UsageBreakdownTooltip", () => {
  it("pairs cache and output Token buckets with their reconciled cost totals", () => {
    const { host, root } = renderTooltip(exactBreakdown());

    expect(
      Array.from(host.querySelectorAll("thead th")).map((header) => header.textContent),
    ).toEqual(["Model", "Cache write", "Cache read", "Cache hit rate", "Output", "Total"]);
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
});
