import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { UsageBreakdown } from "../../lib/api";
import {
  buildAdaptiveCurrencyTextSpec,
  buildAdaptiveNumberTextSpec,
  buildAdaptivePercentTextSpec,
} from "../shared/adaptiveMetricValueSpec";
import { UsageBreakdownTooltip } from "./UsageBreakdownTooltip";

const labels = {
  total: "Total",
  model: "Model",
  cacheWrite: "Cache write",
  cacheRead: "Cache read",
  cacheHitRate: "Cache hit rate",
  cacheHitRateCompact: "Hit rate",
  output: "Output",
  unknownModel: "Unidentified model",
  reasoningEffort: "Reasoning effort",
  tokenUnit: "tokens",
};

const exactBreakdown: UsageBreakdown = {
  cacheWriteTokens: 432_000,
  cacheReadTokens: 196_000,
  outputTokens: 214_190,
  costs: {
    input: 1.96,
    cacheWrite: 3.24,
    cacheRead: 0.44,
    output: 6.12,
    reasoning: 0.71,
    unknown: 0,
  },
  models: [
    {
      model: "gpt-5.6",
      reasoningEffort: " MAX ",
      cacheWriteTokens: 290_000,
      cacheReadTokens: 128_000,
      outputTokens: 146_120,
      costs: {
        input: 1.24,
        cacheWrite: 2.11,
        cacheRead: 0.29,
        output: 4.13,
        reasoning: 0.49,
        unknown: 0,
      },
    },
    {
      model: "gpt-6-sol",
      reasoningEffort: "medium",
      cacheWriteTokens: 60_000,
      cacheReadTokens: 28_000,
      outputTokens: 30_000,
      costs: {
        input: 0.26,
        cacheWrite: 0.44,
        cacheRead: 0.06,
        output: 0.85,
        reasoning: 0.1,
        unknown: 0,
      },
    },
    {
      model: "gpt-5.6-luna-2026-07-27",
      reasoningEffort: "ULTRA",
      cacheWriteTokens: 142_000,
      cacheReadTokens: 68_000,
      outputTokens: 68_070,
      costs: {
        input: 0.72,
        cacheWrite: 1.13,
        cacheRead: 0.15,
        output: 1.99,
        reasoning: 0.22,
        unknown: 0,
      },
    },
    {
      model: "gpt-6.1-sol",
      reasoningEffort: "none",
      cacheWriteTokens: 32_000,
      cacheReadTokens: 12_000,
      outputTokens: 18_000,
      costs: {
        input: 0.31,
        cacheWrite: 0.52,
        cacheRead: 0.08,
        output: 0.93,
        reasoning: 0.11,
        unknown: 0,
      },
    },
  ],
};

const historicalBreakdown: UsageBreakdown = {
  ...exactBreakdown,
  costs: { input: 0, cacheWrite: 0, cacheRead: 0, output: 0, reasoning: 0, unknown: 12.47 },
  models: exactBreakdown.models.map((model) => ({
    ...model,
    costs: {
      input: 0,
      cacheWrite: 0,
      cacheRead: 0,
      output: 0,
      reasoning: 0,
      unknown: model.model === "gpt-5.6" ? (model.reasoningEffort === " MAX " ? 5.76 : 3.72) : 2.99,
    },
  })),
};

const missingCostBreakdown: UsageBreakdown = {
  cacheWriteTokens: exactBreakdown.cacheWriteTokens,
  cacheReadTokens: exactBreakdown.cacheReadTokens,
  outputTokens: exactBreakdown.outputTokens,
  models: exactBreakdown.models.map(({ costs: _costs, ...model }) => model),
};

const longValueBreakdown: UsageBreakdown = {
  cacheWriteTokens: 105_769_103_036,
  cacheReadTokens: 306_688_912,
  outputTokens: 11_896_568_147,
  costs: {
    input: 85_755.9564,
    cacheWrite: 103_355.4234,
    cacheRead: 120.2888,
    output: 11_896.568,
    reasoning: 17.4928,
    unknown: 0,
  },
  models: [
    {
      model: "gpt-5.6",
      reasoningEffort: "max",
      cacheWriteTokens: 34_724_382,
      cacheReadTokens: 12_873_216,
      outputTokens: 5_054_843_182,
      costs: {
        input: 34_724.382,
        cacheWrite: 12_873.216,
        cacheRead: 5_054.8432,
        output: 2_527.4216,
        reasoning: 742.19,
        unknown: 0,
      },
    },
    {
      model: "gpt-6-luna",
      reasoningEffort: "medium",
      cacheWriteTokens: 37_212_246,
      cacheReadTokens: 944_699_008,
      outputTokens: 3_608_133,
      costs: {
        input: 37_212.246,
        cacheWrite: 944_699.008,
        cacheRead: 3_608.133,
        output: 1_804.0665,
        reasoning: 380.25,
        unknown: 0,
      },
    },
    {
      model: "gpt-6.1-sol",
      reasoningEffort: "none",
      cacheWriteTokens: 33_232_552,
      cacheReadTokens: 918_523_776,
      outputTokens: 3_163_948,
      costs: {
        input: 33_232.552,
        cacheWrite: 918_523.776,
        cacheRead: 3_163.948,
        output: 1_581.974,
        reasoning: 310.12,
        unknown: 0,
      },
    },
  ],
};

function buildNumberSpec(value: number) {
  return buildAdaptiveNumberTextSpec(value, "en-US", 0);
}

function buildRatioSpec(value: number | null) {
  return buildAdaptivePercentTextSpec(value, "en-US", { maximumFractionDigits: 1 });
}

function buildCurrencySpec(value: number | null) {
  return buildAdaptiveCurrencyTextSpec(value, "en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  });
}

const meta = {
  title: "Dashboard/UsageBreakdownTooltip",
  component: UsageBreakdownTooltip,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider initialLocale="en" persistLocale={false}>
        <Story />
      </I18nProvider>
    ),
  ],
  args: {
    title: "Usage details",
    breakdown: exactBreakdown,
    buildNumberSpec,
    buildRatioSpec,
    buildCurrencySpec,
    labels,
  },
  render: (args) => (
    <div
      data-visual-evidence-surface="dashboard-usage-breakdown-tooltip"
      className="w-[692px] max-w-full bg-base-200 px-[18px] py-6 text-base-content sm:px-6"
    >
      <div
        data-visual-evidence-target="dashboard-usage-breakdown-table"
        className="mx-auto w-full max-w-[48rem]"
      >
        <UsageBreakdownTooltip {...args} />
      </div>
    </div>
  ),
} satisfies Meta<typeof UsageBreakdownTooltip>;

export default meta;

type Story = StoryObj<typeof meta>;

export const ExactCosts: Story = {
  tags: ["test"],
  args: { breakdown: exactBreakdown },
  play: async ({ canvasElement }) => {
    const desktopTable = canvasElement.querySelector(
      '[data-testid="usage-breakdown-table-scroll-region"]',
    );
    if (!(desktopTable instanceof HTMLElement)) throw new Error("missing usage table");
    const identities = Array.from(desktopTable.querySelectorAll<HTMLElement>("tbody tr"))
      .map((row) => [
        row.querySelector<HTMLElement>("[data-model-identity]")?.dataset.modelIdentity,
        row.querySelector<HTMLElement>("[data-model-generation-segment]")?.textContent,
      ])
      .filter(([model]) => model !== undefined);
    await expect(identities).toEqual(
      expect.arrayContaining([
        ["gpt-5.6", "5.6"],
        ["gpt-5.6-luna-2026-07-27", "5.6"],
        ["gpt-6-sol", "6"],
        ["gpt-6.1-sol", "6.1"],
      ]),
    );
    for (const row of desktopTable.querySelectorAll<HTMLElement>("tbody tr")) {
      await expect(row.scrollWidth).toBeLessThanOrEqual(row.clientWidth);
      const badge = row.querySelector<HTMLElement>("[data-model-identity-badge]");
      if (badge) {
        await expect(badge.getBoundingClientRect().height).toBe(24);
        const segments = Array.from(
          badge.querySelectorAll(
            "[data-model-generation-segment], [data-model-icon-segment], [data-model-effort-segment]",
          ),
        );
        await expect(segments).toHaveLength(3);
        await expect(segments[0]).toHaveAttribute("data-model-generation-segment");
        await expect(segments[1]).toHaveAttribute("data-model-icon-segment");
        await expect(segments[2]).toHaveAttribute("data-model-effort-segment");
      }
    }
  },
};

export const SimpleGrouping: Story = {
  tags: ["test"],
  args: { breakdown: exactBreakdown },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("dashboard-model-breakdown-mode-simple"));
    await expect(canvas.getByTestId("dashboard-model-breakdown-mode-simple")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(canvas.getAllByRole("row")).toHaveLength(6);
    await expect(canvasElement.querySelector("select")).toBeNull();
    await expect(
      canvasElement.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-write"]'),
    ).toBeNull();
    await expect(
      canvasElement.querySelector('[data-testid="dashboard-model-breakdown-sort-cache-read"]'),
    ).toBeNull();
    await expect(
      canvasElement.querySelector('[data-testid="dashboard-model-breakdown-sort-output"]'),
    ).toBeNull();
    await expect(
      canvas.getAllByTestId("dashboard-model-breakdown-sort-total").length,
    ).toBeGreaterThan(0);
  },
};

export const HistoricalTotalOnly: Story = {
  args: { breakdown: historicalBreakdown },
};

export const MissingCostDetails: Story = {
  args: { breakdown: missingCostBreakdown },
};

export const Mobile390: Story = {
  tags: ["test"],
  ...ExactCosts,
  args: {
    breakdown: longValueBreakdown,
  },
  globals: {
    viewport: { value: "mobile390", isRotated: false },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("dashboard-model-breakdown-mode-detailed"));
    const mobileList = canvas.getByTestId("usage-breakdown-mobile-list");
    const mobileCanvas = within(mobileList);
    await expect(mobileList).toBeInTheDocument();
    const mobileIdentities = Array.from(
      mobileList.querySelectorAll<HTMLElement>("[data-model-identity]"),
    );
    const identityPairs = mobileIdentities.map((identity) => [
      identity.dataset.modelIdentity,
      identity
        .closest<HTMLElement>("[data-model-identity-badge]")
        ?.querySelector<HTMLElement>("[data-model-generation-segment]")?.textContent,
    ]);
    await expect(identityPairs).toEqual(
      expect.arrayContaining([
        ["gpt-5.6", "5.6"],
        ["gpt-6-luna", "6"],
        ["gpt-6.1-sol", "6.1"],
      ]),
    );
    await expect(mobileList.querySelector('[data-model-identity="gpt-6-luna"]')).toHaveAttribute(
      "data-model-icon",
      "weather-night",
    );
    await expect(mobileList.querySelector('[data-model-identity="gpt-6.1-sol"]')).toHaveAttribute(
      "data-model-icon",
      "white-balance-sunny",
    );
    await expect(mobileCanvas.getByTestId("usage-breakdown-mobile-controls")).toBeInTheDocument();
    await expect(
      mobileCanvas.getByTestId("dashboard-model-breakdown-sort-total"),
    ).toBeInTheDocument();
    const modelSort = mobileCanvas.getByTestId("dashboard-model-breakdown-model-sort");
    const totalSort = mobileCanvas.getByTestId("dashboard-model-breakdown-sort-total");
    const modelLabel = modelSort.querySelector("span");
    if (!(modelLabel instanceof HTMLElement)) {
      throw new Error("missing mobile model sort label");
    }
    await expect(getComputedStyle(modelLabel).lineHeight).toBe("16px");
    for (const element of mobileList.querySelectorAll("button, section > div, dt, dd")) {
      await expect(Number.parseFloat(getComputedStyle(element).fontSize)).toBeGreaterThanOrEqual(
        12,
      );
    }
    for (const label of mobileList.querySelectorAll("section:not(:first-child) > div > span")) {
      await expect(getComputedStyle(label).flexDirection).toBe("row");
    }
    await expect(
      Math.abs(modelSort.getBoundingClientRect().top - totalSort.getBoundingClientRect().top),
    ).toBeLessThanOrEqual(1);
    await expect(mobileList.scrollWidth).toBeLessThanOrEqual(mobileList.clientWidth);
    await expect(canvas.getByTestId("usage-breakdown-table-scroll-region")).toHaveClass("hidden");
    const compactValue = mobileList.querySelector<HTMLElement>(
      '[data-adaptive-metric-visible="true"][data-compact="true"]',
    );
    if (!compactValue) throw new Error("missing compact mobile usage value");
    await expect(compactValue).not.toHaveAttribute("title");
    await userEvent.click(compactValue);
    await expect(canvasElement.ownerDocument.body.textContent).toContain("105,769,103,036 tokens");
  },
};

export const ConstrainedOverlay: Story = {
  ...ExactCosts,
  args: { breakdown: longValueBreakdown },
  render: (args) => (
    <div
      data-visual-evidence-surface="dashboard-usage-breakdown-constrained-overlay"
      className="w-[692px] max-w-full bg-base-200 px-4 py-6 text-base-content sm:px-6"
    >
      <div
        data-visual-evidence-target="dashboard-usage-breakdown-constrained-table"
        data-testid="usage-breakdown-constrained-overlay"
        className="w-[644px] max-w-full px-3.5 py-3"
      >
        <UsageBreakdownTooltip {...args} />
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const overlay = canvasElement.querySelector(
      '[data-testid="usage-breakdown-constrained-overlay"]',
    );
    if (!(overlay instanceof HTMLElement)) {
      throw new Error("missing constrained usage breakdown overlay");
    }
    await expect(overlay.scrollWidth).toBeLessThanOrEqual(overlay.clientWidth);
    const tableRegion = canvasElement.querySelector(
      '[data-testid="usage-breakdown-table-scroll-region"]',
    );
    if (!(tableRegion instanceof HTMLElement)) {
      throw new Error("missing usage breakdown table region");
    }
    await expect(
      Number.parseFloat(getComputedStyle(tableRegion.querySelector("table")!).fontSize),
    ).toBeGreaterThanOrEqual(12);
    await expect(tableRegion.scrollWidth).toBeLessThanOrEqual(tableRegion.clientWidth);
    for (const header of tableRegion.querySelectorAll<HTMLTableCellElement>("thead th")) {
      await expect(header.scrollWidth).toBeLessThanOrEqual(header.clientWidth);
    }
    for (const headerButton of tableRegion.querySelectorAll<HTMLButtonElement>("thead button")) {
      await expect(headerButton.scrollWidth).toBeLessThanOrEqual(headerButton.clientWidth);
      await expect(getComputedStyle(headerButton).whiteSpace).toBe("nowrap");
    }
    for (const headerLabel of tableRegion.querySelectorAll<HTMLElement>("thead th > span")) {
      await expect(getComputedStyle(headerLabel).whiteSpace).toBe("nowrap");
    }
    const compactValue = tableRegion.querySelector<HTMLElement>(
      '[data-adaptive-metric-visible="true"][data-compact="true"]',
    );
    if (!compactValue) throw new Error("missing compact desktop usage value");
    await expect(compactValue).not.toHaveAttribute("title");
    await userEvent.click(compactValue);
    await expect(canvasElement.ownerDocument.body.textContent).toContain("105,769,103,036 tokens");
  },
};
