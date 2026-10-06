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
const tasks = managedTasks() as ManagedTask[];
const task = tasks.find((item) => item.taskKey === "retention_archive") ?? tasks[0];
const emptyTask = tasks.find((item) => item.taskKey === "summary_snapshot") ?? task;
const failedTask = tasks.find((item) => item.taskKey === "long_term_projection") ?? task;

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
  tags: ["autodocs"],
  parameters: { layout: "padded" },
  decorators: [
    (Story: () => ReactNode) => (
      <StorybookPageEnvironment
        onRequest={workloadHandler({
          retention_archive: fixture,
          summary_snapshot: emptyTrend,
          long_term_projection: response(emptyTrend, 503),
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
    await expect(canvasElement.querySelector("svg")).not.toBeNull();
    await userEvent.click(canvas.getByRole("button", { name: "查看运行计量" }));
    await expect(canvas.getByRole("dialog", { name: /运行计量详情/ })).toHaveTextContent(
      "触发时间：",
    );
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
    await expect(canvasElement.querySelector("svg")).not.toBeNull();
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
