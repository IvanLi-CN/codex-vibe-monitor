import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, within } from "storybook/test";
import { managedTasks } from "../../demo/handlers";
import type {
  CurrentTaskExecution,
  ManagedTask,
  TaskTimelineCoverage,
  TaskTimelineSegment,
} from "../../lib/api";
import { TaskTimelineChart } from "./TaskTimelineChart";

const NOW = Date.now();
const TASKS = managedTasks() as ManagedTask[];
const runtimeUid = "story-active-retention";
const activeRun: CurrentTaskExecution = {
  executionId: 1,
  executionUid: runtimeUid,
  taskKey: "retention_archive",
  title: "数据保留与归档",
  activeChildTaskKey: "archive",
  activeChildTitle: "归档批次",
  triggerKind: "interval",
  phase: "processing",
  executionClass: "maintenance_retention",
  startedAt: new Date(NOW - 8 * 60_000).toISOString(),
  elapsedMs: 8 * 60_000,
};

function segment(
  overrides: Partial<TaskTimelineSegment> &
    Pick<TaskTimelineSegment, "segmentId" | "taskKey" | "startedAt">,
): TaskTimelineSegment {
  const task = TASKS.find((item) => item.taskKey === overrides.taskKey);
  return {
    kind: "execution",
    title: task?.title ?? overrides.taskKey,
    lastObservedAt: overrides.finishedAt ?? overrides.startedAt,
    finishedAt: null,
    durationMs: null,
    status: "success",
    triggerKind: "interval",
    executionClass: task?.executionClass ?? null,
    reason: null,
    retryAt: null,
    activeChildTaskKey: null,
    activeChildTitle: null,
    managedRunId: null,
    sessionId: "storybook-session",
    revision: 1,
    ...overrides,
  };
}

const mixedSegments: TaskTimelineSegment[] = [
  segment({
    segmentId: runtimeUid,
    taskKey: "retention_archive",
    startedAt: activeRun.startedAt,
    lastObservedAt: new Date(NOW).toISOString(),
    status: "running",
    managedRunId: 31,
  }),
  segment({
    segmentId: "failure-run",
    taskKey: "raw_payload_metrics_inventory",
    startedAt: new Date(NOW - 90 * 60_000).toISOString(),
    lastObservedAt: new Date(NOW - 89 * 60_000).toISOString(),
    finishedAt: new Date(NOW - 89 * 60_000).toISOString(),
    durationMs: 60_000,
    status: "failed",
    managedRunId: 29,
  }),
  segment({
    segmentId: "interrupted-run",
    taskKey: "startup_backfill.proxy_usage",
    startedAt: new Date(NOW - 6 * 60 * 60_000).toISOString(),
    lastObservedAt: new Date(NOW - 5.9 * 60 * 60_000).toISOString(),
    status: "interrupted",
    triggerKind: "event",
  }),
  segment({
    segmentId: "resource-wait",
    kind: "deferral",
    taskKey: "timeseries_minute_projection",
    startedAt: new Date(NOW - 6 * 60_000).toISOString(),
    lastObservedAt: new Date(NOW - 2 * 60_000).toISOString(),
    finishedAt: new Date(NOW - 2 * 60_000).toISOString(),
    durationMs: 4 * 60_000,
    status: "released",
    reason: "resource_busy",
    triggerKind: null,
  }),
  segment({
    segmentId: "pressure-wait",
    kind: "deferral",
    taskKey: "long_term_projection",
    startedAt: new Date(NOW - 4 * 60_000).toISOString(),
    lastObservedAt: new Date(NOW).toISOString(),
    status: "waiting",
    reason: "pressure_cooldown",
    retryAt: new Date(NOW + 3_000).toISOString(),
    triggerKind: null,
  }),
  segment({
    segmentId: "overflow-gap",
    kind: "coverage_gap",
    taskKey: "__timeline__",
    title: "观测缺口",
    startedAt: new Date(NOW - 80 * 60_000).toISOString(),
    lastObservedAt: new Date(NOW - 78 * 60_000).toISOString(),
    finishedAt: new Date(NOW - 78 * 60_000).toISOString(),
    status: "unknown",
    reason: "event_channel_overflow",
  }),
];

const coverage: TaskTimelineCoverage[] = [
  {
    sessionId: "storybook-session",
    startedAt: new Date(NOW - 21 * 60 * 60_000).toISOString(),
    lastSeenAt: new Date(NOW).toISOString(),
    endedAt: null,
    droppedEvents: 3,
  },
];

const denseSegments = Array.from({ length: 24 }, (_, index) => {
  const startedAt = NOW - 60_000 + index * 1_000;
  return segment({
    segmentId: `dense-${index}`,
    taskKey: "summary_snapshot",
    startedAt: new Date(startedAt).toISOString(),
    lastObservedAt: new Date(startedAt + 2_000).toISOString(),
    finishedAt: new Date(startedAt + 2_000).toISOString(),
    durationMs: 2_000,
    status: index % 8 === 0 ? "failed" : "success",
    triggerKind: "event",
  });
});

const baseArgs = {
  tasks: TASKS,
  executions: mixedSegments,
  activeRuns: [activeRun],
  coverage,
  nowMs: NOW,
  runtimeFresh: true,
  runtimeBoundaryMs: NOW,
  runtimeObservedAt: new Date(NOW).toISOString(),
};

function withTheme(theme: "vibe-light" | "vibe-dark") {
  return (Story: () => ReactNode) => {
    const previousTheme = document.documentElement.getAttribute("data-theme");
    const previousMode = document.documentElement.getAttribute("data-color-mode");
    document.documentElement.setAttribute("data-theme", theme);
    document.documentElement.setAttribute(
      "data-color-mode",
      theme === "vibe-dark" ? "dark" : "light",
    );
    return (
      <MemoryRouter>
        <div data-visual-evidence-surface className="bg-base-100 p-6 text-base-content">
          <div data-visual-evidence-target>
            <Story />
          </div>
        </div>
        <ThemeCleanup previousTheme={previousTheme} previousMode={previousMode} />
      </MemoryRouter>
    );
  };
}

function ThemeCleanup({
  previousTheme,
  previousMode,
}: {
  previousTheme: string | null;
  previousMode: string | null;
}): null {
  useEffect(() => {
    return () => {
      if (previousTheme) document.documentElement.setAttribute("data-theme", previousTheme);
      else document.documentElement.removeAttribute("data-theme");
      if (previousMode) document.documentElement.setAttribute("data-color-mode", previousMode);
      else document.documentElement.removeAttribute("data-color-mode");
    };
  }, [previousMode, previousTheme]);
  return null;
}

const meta = {
  title: "System/TaskTimelineChart",
  component: TaskTimelineChart,
  tags: ["autodocs"],
  parameters: { layout: "padded" },
  decorators: [withTheme("vibe-light")],
} satisfies Meta<typeof TaskTimelineChart>;

export default meta;
type Story = StoryObj<typeof meta>;

export const MixedOperations: Story = {
  args: baseArgs,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const pressureBars = canvas.getAllByRole("button", {
      name: /资源占用等待/,
    });
    await expect(pressureBars).toHaveLength(2);
    const yCoordinates = pressureBars.map((bar) => bar.querySelector("rect")?.getAttribute("y"));
    await expect(new Set(yCoordinates).size).toBe(1);
  },
};

export const DarkTheme: Story = {
  args: baseArgs,
  decorators: [withTheme("vibe-dark")],
};

export const DenseShortRuns: Story = {
  args: { ...baseArgs, executions: denseSegments, activeRuns: [] },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const executionButtons = canvas.getAllByRole("button", { name: /汇总快照/ });
    const groups = executionButtons.filter((button) =>
      (button.getAttribute("aria-label") ?? "").includes("选择查看详情"),
    );
    const individualRuns = executionButtons.filter(
      (button) => !(button.getAttribute("aria-label") ?? "").includes("选择查看详情"),
    );
    await expect(groups.length).toBeGreaterThan(0);
    let inspectedRuns = 0;
    for (const [button, count] of [
      ...groups.map(
        (button) =>
          [
            button,
            Number((button.getAttribute("aria-label") ?? "").match(/^(\d+) 次/)?.[1]),
          ] as const,
      ),
      ...individualRuns.map((button) => [button, 1] as const),
    ]) {
      await userEvent.click(button);
      await expect(canvas.getByText(`${count} 次任务执行`)).toBeVisible();
      const details = canvas.getByRole("list", { name: "执行记录详情" });
      const runs = within(details).getAllByRole("listitem");
      await expect(runs).toHaveLength(count);
      inspectedRuns += runs.length;
      await userEvent.click(canvas.getByRole("button", { name: "收起" }));
    }
    await expect(inspectedRuns).toBe(24);
  },
};

export const ObservationGap: Story = {
  args: {
    ...baseArgs,
    executions: [],
    activeRuns: [],
    coverage: [],
    runtimeFresh: false,
    runtimeBoundaryMs: NOW,
  },
};

export const CoveredIdle: Story = {
  args: {
    ...baseArgs,
    executions: [],
    activeRuns: [],
  },
};
