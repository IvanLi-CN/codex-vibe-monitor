/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { AdaptiveDisplayValue, AdaptiveMetricValue } from "./AdaptiveMetricValue";
import {
  buildAdaptiveCurrencyAmountTextSpec,
  buildAdaptiveDurationTextSpec,
  buildAdaptiveNumberTextSpec,
} from "./adaptiveMetricValueSpec";

vi.mock("./AnimatedDigits", () => ({
  AnimatedDigits: ({ value }: { value: number | string }) => (
    <span data-testid="animated-digits">{String(value)}</span>
  ),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let metricContainerWidth = 320;
let metricMeasureWidths = new Map<string, number>();

class MockResizeObserver {
  static instances = new Set<MockResizeObserver>();

  private readonly callback: ResizeObserverCallback;
  private readonly observed = new Set<Element>();

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    MockResizeObserver.instances.add(this);
  }

  observe(target: Element) {
    this.observed.add(target);
  }

  unobserve(target: Element) {
    this.observed.delete(target);
  }

  disconnect() {
    this.observed.clear();
    MockResizeObserver.instances.delete(this);
  }

  static notify(target: Element) {
    for (const instance of MockResizeObserver.instances) {
      if (!instance.observed.has(target)) continue;
      instance.callback(
        [{ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry],
        instance as unknown as ResizeObserver,
      );
    }
  }
}

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });

  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });

  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricContainer === "true") {
        return metricContainerWidth;
      }
      return 0;
    },
  });

  Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricMeasure === "true") {
        const text = (this as HTMLElement).textContent ?? "";
        return (
          metricMeasureWidths.get(text) ??
          metricMeasureWidths.get(
            (this as HTMLElement).dataset.adaptiveMetricMeasureIndex ?? "0",
          ) ??
          0
        );
      }
      return 0;
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
  metricContainerWidth = 320;
  metricMeasureWidths = new Map([["0", 120]]);
  MockResizeObserver.instances.clear();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function getMetric() {
  const metric = host?.querySelector('[data-testid="adaptive-metric"]');
  if (!(metric instanceof HTMLElement)) {
    throw new Error("Missing adaptive metric");
  }
  return metric;
}

function getVisibleMetricText() {
  const visible = host?.querySelector('[data-adaptive-metric-visible="true"]');
  if (!(visible instanceof HTMLElement)) {
    throw new Error("Missing visible metric element");
  }
  return visible.textContent ?? "";
}

function getMeasure() {
  const measure = host?.querySelector('[data-adaptive-metric-measure="true"]');
  if (!(measure instanceof HTMLElement)) {
    throw new Error("Missing measure element");
  }
  return measure;
}

describe("AdaptiveMetricValue", () => {
  it("prioritizes account-card compact values only after the hard grouping threshold", () => {
    const compactSpec = buildAdaptiveNumberTextSpec(30_030_779, "en-US", 0, {
      presentation: "account-stat-card",
    });
    const ordinarySpec = buildAdaptiveNumberTextSpec(10_376, "en-US", 0, {
      presentation: "account-stat-card",
    });
    const defaultSpec = buildAdaptiveNumberTextSpec(30_030_779, "en-US", 0);

    expect(compactSpec.candidates[0]?.value).toBe("30.0M");
    expect(compactSpec.fullValue).toBe("30,030,779");
    expect(ordinarySpec.candidates[0]?.value).toBe("10,376");
    expect(defaultSpec.candidates[0]?.value).toBe("30,030,779");
  });

  it("keeps the exact full value accessible when an account card starts compact", () => {
    const spec = buildAdaptiveNumberTextSpec(30_030_779, "en-US", 0, {
      presentation: "account-stat-card",
    });
    metricContainerWidth = 120;
    metricMeasureWidths = new Map([
      ["30.0M", 60],
      ["30.031M", 80],
      ["30.03M", 72],
      ["30M", 48],
      ["30,030,779", 180],
    ]);

    render(<AdaptiveDisplayValue spec={spec} data-testid="adaptive-metric" />);

    expect(getVisibleMetricText()).toBe("30.0M");
    expect(getMetric().getAttribute("title")).toBe("30,030,779");
    expect(getMetric().getAttribute("data-compact")).toBe("true");
  });

  it("does not lock an account card to the full candidate while measurements are unavailable", () => {
    const spec = buildAdaptiveNumberTextSpec(30_030_779, "en-US", 0, {
      presentation: "account-stat-card",
    });
    metricContainerWidth = 76;
    metricMeasureWidths = new Map();

    render(<AdaptiveDisplayValue spec={spec} data-testid="adaptive-metric" />);

    metricMeasureWidths = new Map([
      ["30.0M", 60],
      ["30.031M", 80],
      ["30.03M", 72],
      ["30M", 48],
      ["30,030,779", 180],
    ]);
    act(() => {
      MockResizeObserver.notify(getMeasure());
    });

    expect(getVisibleMetricText()).toBe("30.0M");
    expect(getMetric().getAttribute("data-compact")).toBe("true");
  });

  it("upgrades a compact value when rounding crosses its unit boundary", () => {
    const spec = buildAdaptiveNumberTextSpec(999_950, "en-US", 0, {
      presentation: "account-stat-card",
    });

    expect(spec.candidates.some((candidate) => candidate.value === "1.00M")).toBe(true);
    expect(spec.candidates.some((candidate) => candidate.value.includes("1000K"))).toBe(false);
  });

  it("uses the account-card duration ladder without changing the default duration contract", () => {
    expect(
      buildAdaptiveDurationTextSpec(9_300, "en-US", { presentation: "account-stat-card" })
        .fullValue,
    ).toBe("9.3 s");
    expect(
      buildAdaptiveDurationTextSpec(65_000, "en-US", { presentation: "account-stat-card" })
        .fullValue,
    ).toBe("1.08 min");
    expect(
      buildAdaptiveDurationTextSpec(3_600_000, "en-US", { presentation: "account-stat-card" })
        .fullValue,
    ).toBe("1 h");
    expect(buildAdaptiveDurationTextSpec(65_000, "en-US").fullValue).toBe("65 s");
  });

  it("uses dollar-prefixed compact candidates for account-card currency amounts", () => {
    const spec = buildAdaptiveCurrencyAmountTextSpec(30_030_779, "en-US", {
      presentation: "account-stat-card",
    });

    expect(spec.candidates[0]?.value).toBe("$30.0M");
    expect(spec.fullValue).toBe("30,030,779.00");
  });

  it("switches to compact notation when the measured text widens without a container resize", () => {
    metricMeasureWidths = new Map([
      ["1,314,275,579", 120],
      ["1.3143B", 96],
      ["1.314B", 92],
      ["1.31B", 88],
      ["1.3B", 80],
      ["1B", 72],
    ]);

    render(
      <AdaptiveMetricValue value={1314275579} localeTag="en-US" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("false");

    metricMeasureWidths.set("1,314,275,579", 400);
    act(() => {
      MockResizeObserver.notify(getMeasure());
    });

    expect(getMetric().dataset.compact).toBe("true");
    expect(getVisibleMetricText()).toContain("1.31B");
    expect(getMetric().dataset.compactPrecision).toBe("2");
    expect(getMetric().getAttribute("title")).toBe("1,314,275,579");
    expect(host?.querySelector('[data-testid="animated-digits"]')).toBeNull();
  });

  it("re-evaluates overflow on window resize even when ResizeObserver is available", () => {
    metricMeasureWidths = new Map([
      ["1,314,275,579", 120],
      ["1.3143B", 96],
      ["1.314B", 92],
      ["1.31B", 88],
      ["1.3B", 80],
      ["1B", 72],
    ]);

    render(
      <AdaptiveMetricValue value={1314275579} localeTag="en-US" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("false");

    metricContainerWidth = 80;
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });

    expect(getMetric().dataset.compact).toBe("true");
  });

  it("keeps the short-scale compact suffix for zh overflow fallback", () => {
    metricContainerWidth = 100;
    metricMeasureWidths = new Map([
      ["1,314,275,579", 400],
      ["1.3143B", 120],
      ["1.314B", 112],
      ["1.31B", 104],
      ["1.3B", 88],
      ["1B", 72],
    ]);

    render(
      <AdaptiveMetricValue value={1314275579} localeTag="zh-CN" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("true");
    expect(getVisibleMetricText()).toContain("1.3B");
    expect(getVisibleMetricText()).toContain("B");
  });

  it("keeps AnimatedDigits only for non-compact number rendering", () => {
    metricMeasureWidths = new Map([
      ["12,345", 120],
      ["12.35K", 96],
      ["12.3K", 88],
      ["12K", 72],
    ]);

    render(<AdaptiveMetricValue value={12345} localeTag="en-US" data-testid="adaptive-metric" />);

    expect(host?.querySelector('[data-testid="animated-digits"]')?.textContent).toBe("12,345");

    metricContainerWidth = 80;
    metricMeasureWidths.set("12,345", 240);
    act(() => {
      MockResizeObserver.notify(getMeasure());
    });

    expect(getMetric().dataset.compact).toBe("true");
    expect(host?.querySelector('[data-testid="animated-digits"]')).toBeNull();
  });

  it("drops compact decimal precision to preserve the magnitude suffix when width is very tight", () => {
    metricContainerWidth = 76;
    metricMeasureWidths = new Map([
      ["281,110,000", 220],
      ["281.11M", 98],
      ["281.1M", 86],
      ["281M", 58],
    ]);

    render(
      <AdaptiveMetricValue value={281_110_000} localeTag="en-US" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("true");
    expect(getMetric().dataset.candidateKey).toBe("compact-M-0");
    expect(getVisibleMetricText()).toContain("281M");
    expect(getVisibleMetricText()).not.toContain("281.11M");
    expect(getMetric().getAttribute("title")).toBe("281,110,000");
  });

  it("prefers higher-information billion candidates over rounding down to 1B when width allows", () => {
    metricContainerWidth = 96;
    metricMeasureWidths = new Map([
      ["1,049,600,000", 220],
      ["1.0496B", 120],
      ["1.05B", 84],
      ["1.1B", 76],
      ["1B", 68],
      ["1,049.6M", 112],
      ["1,050M", 104],
    ]);

    render(
      <AdaptiveMetricValue value={1_049_600_000} localeTag="en-US" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("true");
    expect(getVisibleMetricText()).toContain("1.05B");
    expect(getVisibleMetricText()).not.toBe("1B");
  });

  it("preserves the minimum billion decimal when the chosen compact candidate would otherwise collapse to 1B", () => {
    metricContainerWidth = 76;
    metricMeasureWidths = new Map([
      ["1,049,600,000", 220],
      ["1.0496B", 120],
      ["1.05B", 94],
      ["1.1B", 86],
      ["1.0B", 60],
      ["1,049.6M", 96],
      ["1,050M", 90],
    ]);

    render(
      <AdaptiveMetricValue value={1_049_600_000} localeTag="en-US" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("true");
    expect(getMetric().dataset.candidateKey).toBe("compact-B-1");
    expect(getVisibleMetricText()).toBe("1.0B");
    expect(getVisibleMetricText()).not.toBe("1B");
  });

  it("falls back to the neighboring compact unit when that preserves more information than 1B", () => {
    metricContainerWidth = 76;
    metricMeasureWidths = new Map([
      ["1,049,600,000", 220],
      ["1.0496B", 120],
      ["1.05B", 94],
      ["1.1B", 86],
      ["1B", 64],
      ["1,049.6M", 96],
      ["1,050M", 60],
    ]);

    render(
      <AdaptiveMetricValue value={1_049_600_000} localeTag="en-US" data-testid="adaptive-metric" />,
    );

    expect(getMetric().dataset.compact).toBe("true");
    expect(getVisibleMetricText()).toContain("1,050M");
    expect(getVisibleMetricText()).not.toBe("1B");
  });

  it("keeps a stable candidate when repeated resize observations stay near the same threshold", () => {
    metricContainerWidth = 98;
    metricMeasureWidths = new Map([
      ["$0.84", 76],
      ["$0.8", 68],
      ["$1", 64],
    ]);

    render(
      <AdaptiveMetricValue
        value={0.84}
        localeTag="en-US"
        kind="currency"
        currencyProfile="rate"
        data-testid="adaptive-metric"
      />,
    );

    expect(getVisibleMetricText()).toBe("$0.84");

    act(() => {
      MockResizeObserver.notify(getMeasure());
      MockResizeObserver.notify(getMeasure());
    });

    expect(getVisibleMetricText()).toBe("$0.84");

    metricContainerWidth = 70;
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });

    expect(getVisibleMetricText()).toBe("$0.8");

    metricContainerWidth = 72;
    act(() => {
      MockResizeObserver.notify(getMeasure());
    });

    expect(getVisibleMetricText()).toBe("$0.8");

    metricContainerWidth = 76;
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });

    expect(getVisibleMetricText()).toBe("$0.84");
  });

  it("uses a two-decimal full candidate for rate currency when width allows", () => {
    metricMeasureWidths = new Map([
      ["$1.00", 72],
      ["$1.0", 64],
      ["$1", 56],
    ]);

    render(
      <AdaptiveMetricValue
        value={1}
        localeTag="en-US"
        kind="currency"
        currencyProfile="rate"
        data-testid="adaptive-metric"
      />,
    );

    expect(getVisibleMetricText()).toBe("$1.00");
    expect(getMetric().dataset.compact).toBe("false");
  });

  it("allows default currency rendering to keep the existing non-padded full candidate", () => {
    metricMeasureWidths = new Map([
      ["$12.4", 72],
      ["$12.4", 72],
      ["$12", 56],
    ]);

    render(
      <AdaptiveMetricValue
        value={12.4}
        localeTag="en-US"
        kind="currency"
        data-testid="adaptive-metric"
      />,
    );

    expect(getVisibleMetricText()).toBe("$12.4");
    expect(getMetric().dataset.compact).toBe("false");
  });

  it("does not drop decimals when the self-measured container exactly fits the full candidate", () => {
    metricContainerWidth = 39;
    metricMeasureWidths = new Map([
      ["0.77", 39],
      ["0.8", 29],
      ["1", 10],
    ]);

    render(
      <AdaptiveDisplayValue
        spec={{
          fullValue: "0.77",
          candidates: [
            {
              key: "full",
              value: "0.77",
              compact: false,
              precisionLabel: "full",
              priority: 0,
            },
            {
              key: "standard-1",
              value: "0.8",
              compact: false,
              precisionLabel: "standard-1",
              priority: 1,
            },
            {
              key: "standard-0",
              value: "1",
              compact: false,
              precisionLabel: "standard-0",
              priority: 2,
            },
          ],
        }}
        data-testid="adaptive-metric"
      />,
    );

    expect(getVisibleMetricText()).toBe("0.77");
    expect(getMetric().dataset.compact).toBe("false");
    expect(getMetric().dataset.candidateKey).toBe("full");
  });
});
