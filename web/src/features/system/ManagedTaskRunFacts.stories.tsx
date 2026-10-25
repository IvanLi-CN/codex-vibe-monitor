import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import type { ManagedTaskRun } from "../../lib/api";
import { ManagedTaskRunFacts } from "./ManagedTaskRunFacts";

const completeRun: ManagedTaskRun = {
  id: 866,
  triggerKind: "cron",
  startedAt: "2026-01-01T00:00:00.000Z",
  executionUid: "demo-execution-retention_archive-866",
  actualStartedAt: "2026-01-01T00:22:40.000Z",
  actualFinishedAt: "2026-01-01T00:31:00.000Z",
  executionDurationMs: 500_000,
  schedulerWaitMs: 1_360_000,
  finishedAt: "2026-01-01T00:31:00.000Z",
  durationMs: 1_860_000,
  status: "success",
  completion: "complete",
  details: { resourceWaitMs: 12_000, fileResourceWaitMs: 8_000 },
};

function formatExpectedTimestamp(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    timeZoneName: "short",
  }).format(new Date(value));
}

const meta = {
  title: "System/ManagedTaskRunFacts",
  component: ManagedTaskRunFacts,
  tags: ["autodocs", "test"],
  decorators: [
    (Story) => (
      <div data-visual-evidence-surface className="bg-base-100 p-6 text-base-content">
        <div data-visual-evidence-target>
          <Story />
        </div>
      </div>
    ),
  ],
  parameters: { layout: "padded" },
} satisfies Meta<typeof ManagedTaskRunFacts>;

export default meta;
type Story = StoryObj<typeof meta>;

export const CompleteWithIdentity: Story = {
  args: { run: completeRun },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    for (const timestamp of [
      completeRun.startedAt,
      completeRun.actualStartedAt,
      completeRun.actualFinishedAt,
    ]) {
      await expect(canvas.getByText(formatExpectedTimestamp(timestamp as string))).toBeVisible();
    }
    await expect(canvas.getByText("500.0 s")).toBeVisible();
    await expect(canvas.getByText("1360.0 s")).toBeVisible();
    await expect(canvas.getByText("12.0 s")).toBeVisible();
    await expect(canvas.getByText("8.0 s")).toBeVisible();
    await expect(canvas.getByText(completeRun.executionUid as string)).toBeVisible();
    await expect(canvas.getByRole("button", { name: "复制执行身份" })).toBeVisible();
  },
};

export const Running: Story = {
  args: {
    run: {
      id: 867,
      startedAt: "2026-01-01T00:00:00.000Z",
      executionUid: "demo-execution-retention_archive-867",
      actualStartedAt: "2026-01-01T00:22:40.000Z",
      actualFinishedAt: null,
      executionDurationMs: null,
      schedulerWaitMs: null,
      status: "running",
      details: { resourceWaitMs: null, fileResourceWaitMs: null },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByText("未知").length).toBeGreaterThanOrEqual(3);
    await expect(canvas.getByText("demo-execution-retention_archive-867")).toBeVisible();
  },
};

export const LegacyRecord: Story = {
  args: {
    run: {
      id: 868,
      startedAt: "2026-01-01T00:00:00.000Z",
      finishedAt: "2026-01-01T00:31:00.000Z",
      durationMs: 1_860_000,
      status: "success",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("运行记录 ID：868")).toBeVisible();
    await expect(canvas.getAllByText("未知").length).toBeGreaterThanOrEqual(4);
    await expect(canvas.queryByText("1860.0 s")).toBeNull();
  },
};

export const Mobile: Story = {
  ...CompleteWithIdentity,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};
