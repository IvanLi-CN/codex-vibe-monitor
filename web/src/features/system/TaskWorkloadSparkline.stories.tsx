import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { managedTasks } from "../../demo/handlers";
import type { ManagedTask, TaskWorkloadTrend } from "../../lib/api";
import {
  StorybookPageEnvironment,
  type StorybookRequestHandler,
} from "../../storybook/storybookPageHelpers";
import { TaskWorkloadSparkline } from "./TaskWorkloadSparkline";
import { buildRetentionWorkloadFixture } from "./taskWorkloadFixtures";

const nowMs = Date.now();
const fixture = buildRetentionWorkloadFixture({
  nowMs,
  sampleCount: 24,
  intervalMs: 60 * 60_000,
  finalPending: 3_200,
}).trend;
const emptyTrend: TaskWorkloadTrend = {
  revision: 1,
  coverage: "no recorded attempts",
  samples: [],
  clearanceEstimateReason: "insufficient_samples",
};
const singleSampleTrend: TaskWorkloadTrend = {
  ...fixture,
  samples: [fixture.samples.at(-1)!],
};
const mixedUnitSample = fixture.samples.at(-1)!;
const mixedUnitTrend: TaskWorkloadTrend = {
  ...fixture,
  samples: [
    {
      ...mixedUnitSample,
      discovered: mixedUnitSample.discovered
        ? { ...mixedUnitSample.discovered, unit: "subscription sources" }
        : mixedUnitSample.discovered,
    },
  ],
};
const tasks = managedTasks() as ManagedTask[];
const task = tasks.find((item) => item.taskKey === "retention_archive") ?? tasks[0];
const emptyTask = tasks.find((item) => item.taskKey === "summary_snapshot") ?? task;
const failedTask = tasks.find((item) => item.taskKey === "long_term_projection") ?? task;
const singleSampleTask = { ...task, taskKey: "single_sample_task", title: "单样本任务" };
const mixedUnitTask = { ...task, taskKey: "mixed_unit_task", title: "混合单位任务" };

function response(trend: TaskWorkloadTrend, status = 200): Response {
  return new Response(JSON.stringify(trend), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function workloadHandler(
  values: Record<string, TaskWorkloadTrend | Response>,
): StorybookRequestHandler {
  return ({ url }) => {
    const match = url.pathname.match(/\/managed-tasks\/([^/]+)\/workload$/);
    if (!match) return undefined;
    const value = values[decodeURIComponent(match[1])];
    return value instanceof Response ? value.clone() : response(value ?? emptyTrend);
  };
}

const meta = {
  title: "System/TaskWorkloadSparkline",
  component: TaskWorkloadSparkline,
  tags: ["autodocs", "test"],
  parameters: { layout: "padded" },
  decorators: [
    (Story: () => ReactNode) => (
      <StorybookPageEnvironment
        onRequest={workloadHandler({
          retention_archive: fixture,
          summary_snapshot: emptyTrend,
          long_term_projection: response(emptyTrend, 503),
          single_sample_task: singleSampleTrend,
          mixed_unit_task: mixedUnitTrend,
        })}
      >
        <div data-visual-evidence-surface className="bg-base-100 p-6 text-base-content">
          <div data-visual-evidence-target className="max-w-3xl">
            <Story />
          </div>
        </div>
      </StorybookPageEnvironment>
    ),
  ],
} satisfies Meta<typeof TaskWorkloadSparkline>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Loaded: Story = {
  args: { task, dark: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText(/P /)).toBeVisible());
    await expect(canvas.getByTestId("task-workload-sparkline-retention_archive")).toHaveAttribute(
      "aria-busy",
      "false",
    );
    await waitFor(() => expect(canvasElement.querySelector("svg")).not.toBeNull());
    await userEvent.click(canvas.getByRole("button", { name: "查看运行计量" }));
    await expect(canvas.getByRole("dialog", { name: /运行计量详情/ })).toHaveTextContent(
      "触发时间：",
    );
  },
};

export const BackgroundRow: Story = {
  args: { task, dark: false, mode: "background" },
  render: (args) => (
    <div className="relative grid gap-3 bg-base-100 px-4 py-4 md:grid-cols-5">
      <TaskWorkloadSparkline {...args} />
      <h3 className="relative z-10 font-semibold">{args.task.title}</h3>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.queryByRole("button", { name: "查看运行计量" })).toBeNull();
    const title = canvas.getByRole("heading", { name: task.title });
    await expect(title).toBeVisible();
  },
};

export const LoadedMobile: Story = {
  args: { task, dark: false },
  globals: { viewport: { value: "mobile393", isRotated: false } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText(/P /)).toBeVisible());
    await expect(canvas.getByTestId("task-workload-sparkline-retention_archive")).toHaveAttribute(
      "aria-busy",
      "false",
    );
    await waitFor(() => expect(canvasElement.querySelector("svg")).not.toBeNull());
  },
};

export const Empty: Story = {
  args: { task: emptyTask, dark: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("暂无运行记录")).toBeVisible());
    await expect(canvasElement.querySelector("svg")).toBeNull();
  },
};

export const LoadFailure: Story = {
  args: { task: failedTask, dark: true },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText(/请求失败|加载失败|503/)).toBeVisible());
    await expect(canvas.getByTestId("task-workload-sparkline-long_term_projection")).toBeVisible();
  },
};

export const SingleSample: Story = {
  args: { task: singleSampleTask, dark: false },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText(/P /)).toBeVisible());
    await waitFor(() =>
      expect(canvasElement.querySelectorAll('[data-chart-marker="point"]')).toHaveLength(3),
    );
    const marker = canvasElement.querySelector('[data-chart-marker="point"]');
    if (!marker) throw new Error("Single-sample chart marker is missing");
    await expect(marker).toHaveAttribute("vector-effect", "non-scaling-stroke");
    await expect(marker).toHaveAttribute("stroke-linecap", "round");
    await expect(marker).toHaveAttribute("stroke-width", "4");
  },
};

export const MixedUnits: Story = {
  args: { task: mixedUnitTask, dark: false },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByText("单位分别显示")).toBeVisible());
    await expect(canvasElement.querySelector("svg title")).toHaveTextContent(
      "P:invocation rows，D:subscription sources",
    );
  },
};
