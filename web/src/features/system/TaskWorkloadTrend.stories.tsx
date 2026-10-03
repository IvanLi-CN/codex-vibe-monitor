import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { expect, userEvent, within } from "storybook/test";
import type { TaskWorkloadTrend as WorkloadTrend } from "../../lib/api";
import { TaskWorkloadSummary, TaskWorkloadTrend } from "./TaskWorkloadTrend";
import { buildRetentionWorkloadFixture } from "./taskWorkloadFixtures";

const NOW = Date.now();
const fixture = buildRetentionWorkloadFixture({
  nowMs: NOW,
  sampleCount: 100,
  intervalMs: 60 * 60_000,
  finalPending: 32_000,
  skippedIndices: [10, 22],
  zeroCommitFailureIndices: [9, 21],
});
const samples = fixture.samples;
const backlog = fixture.backlog;
const trend = fixture.trend;

const emptyTrend: WorkloadTrend = {
  revision: 0,
  coverage: "no recorded attempts",
  samples: [],
  clearanceEstimateReason: "insufficient_samples",
};

const captureGapTrend: WorkloadTrend = {
  ...trend,
  samples: samples.slice(-8),
  coverage: "incomplete: recorder coverage gap",
  coverageGaps: [
    {
      id: "story-gap-1",
      startedAt: new Date(Date.parse(samples[11].attemptedAt) + 2_000).toISOString(),
      finishedAt: new Date(Date.parse(samples[12].attemptedAt) - 2_000).toISOString(),
      reason: "event_channel_overflow",
    },
  ],
  processingRatePerSecond: null,
  processingRateWindow: null,
  clearanceEta: null,
  clearanceEstimateReason: "capture_gap",
};

const meta = {
  title: "System/TaskWorkloadTrend",
  component: TaskWorkloadTrend,
  tags: ["autodocs"],
  parameters: { layout: "padded", viewport: { defaultViewport: "desktop1440" } },
  decorators: [
    (Story: () => ReactNode) => (
      <div data-visual-evidence-surface className="bg-base-100 p-8 text-base-content">
        <div data-visual-evidence-target>
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof TaskWorkloadTrend>;

export default meta;
type Story = StoryObj<typeof meta>;

async function expectEmptyChart(canvasElement: HTMLElement, hint: string): Promise<void> {
  const canvas = within(canvasElement);
  await expect(canvas.getByRole("figure", { name: "工作量：单位未知" })).toBeVisible();
  for (const name of ["隐藏待处理量", "隐藏本次发现", "隐藏本次处理"]) {
    await expect(canvas.getByRole("button", { name })).toBeVisible();
  }
  await expect(canvas.getByText(hint)).toBeVisible();
  await expect(canvasElement.querySelector(".recharts-cartesian-grid")).not.toBeNull();
  await expect(canvasElement.querySelector(".recharts-yAxis")).not.toBeNull();
}

export const Overview: Story = {
  args: {
    taskKey: "retention_archive",
    trend,
    retentionTrend: backlog,
    state: "ready",
  },
  render: (args) => (
    <div className="space-y-5">
      <TaskWorkloadSummary trend={args.trend} />
      <TaskWorkloadTrend {...args} />
    </div>
  ),
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const panelArea = canvasElement.querySelector<HTMLElement>(
      "[data-testid=task-workload-panels]",
    );
    if (!panelArea) throw new Error("Task workload panel container is missing");
    const initialPanelHeight = Math.round(panelArea.getBoundingClientRect().height);
    await expect(
      canvas.queryByText(/满足包含关系|范围变化和缺测|不作累加|按原始值重叠于零基线/),
    ).toBeNull();
    await expect(canvas.getByRole("tab", { name: "最近 100 次运行" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(canvas.getByRole("button", { name: "隐藏待处理量" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await userEvent.click(canvas.getByRole("button", { name: "最近 20 次" }));
    await expect(canvas.getByRole("button", { name: "最近 20 次" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await userEvent.click(canvas.getByRole("button", { name: "最近 100 次" }));
    await userEvent.click(canvas.getByRole("button", { name: "隐藏待处理量" }));
    await expect(canvas.getByRole("button", { name: "显示待处理量" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    const backlogTab = canvas.getByRole("tab", { name: "最近 7 天归档积压" });
    await userEvent.click(backlogTab);
    await userEvent.keyboard("{ArrowLeft}");
    await expect(canvas.getByRole("tab", { name: "最近 100 次运行" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await userEvent.click(canvas.getByRole("tab", { name: "最近 7 天归档积压" }));
    await expect(canvas.getByText("待归档数量")).toBeVisible();
    await expect(Math.round(panelArea.getBoundingClientRect().height)).toBe(initialPanelHeight);
  },
};

export const Empty: Story = {
  args: { taskKey: "retention_archive", trend: emptyTrend, retentionTrend: [], state: "ready" },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    await expectEmptyChart(canvasElement, "暂无运行样本");
  },
};

export const LegacyInterface: Story = {
  args: { taskKey: "dashboard_runtime_projection_reconcile", trend: null, state: "ready" },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    await expectEmptyChart(canvasElement, "暂无运行计量");
  },
};

export const CaptureGap: Story = {
  args: { taskKey: "retention_archive", trend: captureGapTrend, state: "ready" },
  render: (args) => (
    <div className="space-y-5">
      <TaskWorkloadSummary trend={args.trend} />
      <TaskWorkloadTrend {...args} />
    </div>
  ),
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("note")).toHaveTextContent("观测缺口");
    await expect(canvas.getByRole("figure", { name: "工作量：invocation rows" })).toBeVisible();
  },
};

export const Loading: Story = {
  args: { taskKey: "retention_archive", state: "loading" },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    await expectEmptyChart(canvasElement, "加载中");
  },
};

export const ErrorState: Story = {
  args: { taskKey: "retention_archive", trend: null, state: "error", error: "维护库暂不可用" },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    await expectEmptyChart(canvasElement, "读取失败：维护库暂不可用");
  },
};

export const DarkTheme: Story = {
  args: { taskKey: "retention_archive", trend, retentionTrend: backlog, state: "ready" },
  globals: { themeMode: "dark" },
};

export const MobileLight: Story = {
  ...Overview,
  parameters: { viewport: { defaultViewport: "mobile393" } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const panelArea = canvasElement.querySelector<HTMLElement>(
      "[data-testid=task-workload-panels]",
    );
    if (!panelArea) throw new Error("Task workload panel container is missing");
    const initialPanelHeight = Math.round(panelArea.getBoundingClientRect().height);
    await expect(canvas.getByRole("button", { name: "最近 20 次" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(canvas.getByRole("figure", { name: "工作量：invocation rows" })).toBeVisible();
    await userEvent.click(canvas.getByRole("tab", { name: "最近 7 天归档积压" }));
    await expect(canvas.getByText("最长逾期")).toBeVisible();
    await expect(Math.round(panelArea.getBoundingClientRect().height)).toBe(initialPanelHeight);
  },
};

export const MobileDark: Story = {
  ...DarkTheme,
  parameters: { viewport: { defaultViewport: "mobile393" } },
};
