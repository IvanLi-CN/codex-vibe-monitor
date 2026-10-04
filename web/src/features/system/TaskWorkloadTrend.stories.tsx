import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { act } from "react";
import { expect, userEvent, within } from "storybook/test";
import type {
  TaskMeasurementCapabilities,
  TaskWorkloadTrend as WorkloadTrend,
} from "../../lib/api";
import { TaskWorkloadSummary, TaskWorkloadTrend } from "./TaskWorkloadTrend";
import { buildRetentionWorkloadFixture } from "./taskWorkloadFixtures";

const NOW = Date.parse("2026-10-04T12:00:00.000Z");
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
const skippedRunTrend = buildRetentionWorkloadFixture({
  nowMs: NOW,
  sampleCount: 15,
  intervalMs: 60 * 60_000,
  finalPending: 32_000,
  skippedIndices: [6],
}).trend;
const runningWithCountersTrend: WorkloadTrend = {
  ...trend,
  samples: trend.samples.map((sample, index) =>
    index === trend.samples.length - 1
      ? {
          ...sample,
          status: "running",
          finishedAt: null,
          reason: null,
        }
      : sample,
  ),
};

const emptyTrend: WorkloadTrend = {
  revision: 0,
  coverage: "no recorded attempts",
  samples: [],
  clearanceEstimateReason: "insufficient_samples",
};

const recoveryCapabilities: TaskMeasurementCapabilities = {
  pending: { supported: false, unit: null, scope: null },
  discovered: { supported: false, unit: null, scope: null },
  processed: { supported: true, unit: "pool attempts", scope: "pool_orphan_recovery" },
};

const runningWithoutCounters: WorkloadTrend = {
  ...emptyTrend,
  coverage: "incomplete: run has no observed counters",
  samples: [
    {
      ...samples[samples.length - 1],
      sampleId: "active-no-counters:pool_orphan_recovery",
      executionUid: "active-no-counters",
      managedRunId: null,
      taskKey: "pool_orphan_recovery",
      attemptedAt: new Date(NOW - 60_000).toISOString(),
      actualStartedAt: new Date(NOW - 60_000).toISOString(),
      finishedAt: null,
      status: "running",
      subsetRelation: "unknown",
      pending: null,
      discovered: null,
      processed: {
        unit: "pool attempts",
        scope: "pool_orphan_recovery",
        value: null,
        range: "not-observed",
        observedAt: null,
        coverage: "unknown",
      },
    },
  ],
};

const captureGapTrend: WorkloadTrend = {
  ...trend,
  samples: samples.slice(-8),
  coverage: "incomplete: recorder coverage gap",
  coverageGaps: [
    {
      id: "story-gap-1",
      startedAt: new Date(Date.parse(samples[92].attemptedAt) + 2_000).toISOString(),
      finishedAt: new Date(Date.parse(samples[93].attemptedAt) - 2_000).toISOString(),
      reason: "event_channel_overflow",
    },
  ],
  processingRatePerSecond: null,
  processingRateWindow: null,
  clearanceEta: null,
  clearanceEstimateReason: "capture_gap",
};

const captureGapOutsideWindowTrend: WorkloadTrend = {
  ...captureGapTrend,
  coverageGaps: [
    {
      id: "story-old-gap",
      startedAt: new Date(Date.parse(samples[10].attemptedAt) + 2_000).toISOString(),
      finishedAt: new Date(Date.parse(samples[11].attemptedAt) - 2_000).toISOString(),
      reason: "event_channel_overflow",
    },
  ],
};

const zeroPendingTrend = (clearanceEstimateReason: string): WorkloadTrend => ({
  ...trend,
  latestPending: trend.latestPending ? { ...trend.latestPending, value: 0 } : null,
  clearanceEta: null,
  clearanceEstimateReason,
});

const meta = {
  title: "System/TaskWorkloadTrend",
  component: TaskWorkloadTrend,
  tags: ["autodocs"],
  parameters: { layout: "padded", viewport: { defaultViewport: "desktop1440" } },
  decorators: [
    (Story: () => ReactNode) => (
      <div data-visual-evidence-surface className="bg-base-100 p-4 md:p-8 text-base-content">
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

export const CaptureGapOutsideVisibleWindow: Story = {
  args: { taskKey: "retention_archive", trend: captureGapOutsideWindowTrend, state: "ready" },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).queryByRole("note")).toBeNull();
  },
};

export const ExactZeroEstimateStates: Story = {
  args: { taskKey: "retention_archive", trend, state: "ready" },
  render: () => (
    <div className="grid gap-4 md:grid-cols-2">
      <TaskWorkloadSummary trend={zeroPendingTrend("stale_observation")} />
      <TaskWorkloadSummary trend={zeroPendingTrend("task_disabled")} />
      <TaskWorkloadSummary trend={zeroPendingTrend("capture_gap")} />
    </div>
  ),
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("观测已过期，等待新数据")).toBeVisible();
    await expect(canvas.getByText("任务已停用，暂停清零预估")).toBeVisible();
    await expect(canvas.getByText("观测存在缺口，清零预估暂不可用")).toBeVisible();
    await expect(canvas.queryByText("已清空")).toBeNull();
  },
};

export const AccessiblePointInspection: Story = {
  args: { taskKey: "retention_archive", trend, state: "ready" },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const chart = canvasElement.querySelector<HTMLElement>('[role="application"]');
    const point = canvasElement.querySelector<SVGCircleElement>("circle.recharts-dot");
    if (!chart || !point) throw new Error("Accessible chart point is missing");

    await userEvent.pointer({ target: point, keys: "[TouchA]" });
    await expect(canvas.getByText(/触发：/)).toBeVisible();
    await userEvent.pointer({ target: point, keys: "[/TouchA]" });

    chart.focus();
    await userEvent.keyboard("{ArrowRight}");
    await expect(canvas.getByText(/触发：/)).toBeVisible();
  },
};

export const SkippedRunInspection: Story = {
  args: { taskKey: "retention_archive", trend: skippedRunTrend, state: "ready" },
  parameters: { viewport: { defaultViewport: "mobile393" } },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const chart = canvasElement.querySelector<HTMLElement>('[role="application"]');
    const marker = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-skip-marker-dot']",
    );
    if (!chart || !marker) throw new Error("Skipped-run marker is missing");

    await userEvent.pointer({ target: marker, keys: "[TouchA]" });
    let detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("确认跳过");
    await expect(detail).toHaveTextContent("资源准入确认跳过");
    await userEvent.pointer({ target: marker, keys: "[/TouchA]" });
    await userEvent.click(canvas.getByRole("button", { name: "关闭运行详情" }));

    const keyboardMarker = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-skip-marker-dot']",
    );
    if (!keyboardMarker) throw new Error("Skipped-run marker was removed");
    await act(async () => keyboardMarker.focus());
    await userEvent.keyboard("{Enter}");
    detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("确认跳过");
    await expect(detail).toHaveTextContent("资源准入确认跳过");
  },
};

export const FailedMeasuredRunInspection: Story = {
  args: { taskKey: "retention_archive", trend, state: "ready" },
  parameters: { viewport: { defaultViewport: "mobile393" } },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const marker = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-failed-marker']",
    );
    if (!marker) throw new Error("Measured failed-run marker is missing");

    await userEvent.pointer({ target: marker, keys: "[TouchA]" });
    let detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("失败");
    await expect(detail).toHaveTextContent(/ invocation rows|invocation rows/);
    await userEvent.pointer({ target: marker, keys: "[/TouchA]" });
    await userEvent.click(canvas.getByRole("button", { name: "关闭运行详情" }));

    const keyboardMarker = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-failed-marker']",
    );
    if (!keyboardMarker) throw new Error("Measured failed-run marker was removed");
    await act(async () => keyboardMarker.focus());
    await userEvent.keyboard("{Enter}");
    detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("失败");
    await expect(detail).toHaveTextContent("部分归档提交后遇到可恢复错误");
  },
};

export const RunningWithCountersInspection: Story = {
  args: { taskKey: "retention_archive", trend: runningWithCountersTrend, state: "ready" },
  parameters: { viewport: { defaultViewport: "mobile393" } },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const marker = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-running-marker']",
    );
    if (!marker) throw new Error("Measured running-run marker is missing");

    await userEvent.pointer({ target: marker, keys: "[TouchA]" });
    let detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("运行中");
    await expect(detail).toHaveTextContent(/待处理量：\s*\d/);
    await userEvent.pointer({ target: marker, keys: "[/TouchA]" });
    await userEvent.click(canvas.getByRole("button", { name: "关闭运行详情" }));

    const keyboardMarker = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-running-marker']",
    );
    if (!keyboardMarker) throw new Error("Measured running-run marker was removed");
    keyboardMarker.focus();
    await userEvent.keyboard("{Enter}");
    detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("运行中");
    await expect(detail).toHaveTextContent("实际开始：");
  },
};

export const LegendAvailabilityFollowsRunWindow: Story = {
  args: {
    taskKey: "retention_archive",
    trend: {
      ...trend,
      samples: trend.samples.map((sample, index) =>
        index < trend.samples.length - 20
          ? sample
          : { ...sample, discovered: null, subsetRelation: "unknown" },
      ),
    },
    state: "ready",
  },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const discoveredLegend = canvas.getByRole("button", { name: "隐藏本次发现" });
    await expect(discoveredLegend).not.toHaveTextContent("暂无观测");
    await userEvent.click(canvas.getByRole("button", { name: "最近 20 次" }));
    await expect(discoveredLegend).toHaveTextContent("本次发现 · 暂无观测");
    await userEvent.click(canvas.getByRole("button", { name: "最近 100 次" }));
    await expect(discoveredLegend).not.toHaveTextContent("暂无观测");
  },
};

export const RunningWithoutCounters: Story = {
  args: {
    taskKey: "pool_orphan_recovery",
    capabilities: recoveryCapabilities,
    trend: runningWithoutCounters,
    state: "ready",
  },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const chart = canvasElement.querySelector<HTMLElement>('[role="application"]');
    const point = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-running-marker']",
    );
    if (!chart || !point) throw new Error("Unmeasured running point is missing");

    await userEvent.pointer({ target: point, keys: "[TouchA]" });
    let detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("运行中");
    await expect(detail).toHaveTextContent("实际开始：");
    await expect(detail).toHaveTextContent("待处理量：");
    await expect(detail).toHaveTextContent("不适用");
    await expect(detail).toHaveTextContent("未知");
    await userEvent.pointer({ target: point, keys: "[/TouchA]" });
    await userEvent.click(canvas.getByRole("button", { name: "关闭运行详情" }));

    const keyboardPoint = canvasElement.querySelector<HTMLButtonElement>(
      "button[data-testid='task-workload-running-marker']",
    );
    if (!keyboardPoint) throw new Error("Unmeasured running point was removed");
    await act(async () => keyboardPoint.focus());
    await userEvent.keyboard("{Enter}");
    detail = canvas.getByTestId("task-workload-run-detail");
    await expect(detail).toBeVisible();
    await expect(detail).toHaveTextContent("运行中");
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

export const MobileLightBacklog: Story = {
  args: { taskKey: "retention_archive", trend, retentionTrend: backlog, state: "ready" },
  parameters: { viewport: { defaultViewport: "mobile393" } },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("tab", { name: "最近 7 天归档积压" }));
    await expect(canvas.getByText("待归档数量")).toBeVisible();
    await expect(canvas.getByText("最长逾期")).toBeVisible();
  },
};

export const MobileDark: Story = {
  ...DarkTheme,
  parameters: { viewport: { defaultViewport: "mobile393" } },
};

export const MobileLightCandidates: Story = {
  args: { taskKey: "retention_archive", trend, retentionTrend: backlog, state: "ready" },
  parameters: { viewport: { defaultViewport: "mobile393" } },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "隐藏待处理量" }));
    await expect(canvas.getByRole("button", { name: "显示待处理量" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    await expect(canvas.getByRole("figure", { name: "工作量：invocation rows" })).toBeVisible();
  },
};

export const MobileRunningWithoutCounters: Story = {
  args: {
    taskKey: "pool_orphan_recovery",
    capabilities: recoveryCapabilities,
    trend: runningWithoutCounters,
    state: "ready",
  },
  parameters: { viewport: { defaultViewport: "mobile393" } },
};
