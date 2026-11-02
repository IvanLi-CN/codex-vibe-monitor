import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { RetentionOwnerRunFacts } from "./RetentionOwnerRunFacts";

const meta = {
  title: "System/RetentionOwnerRunFacts",
  component: RetentionOwnerRunFacts,
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
} satisfies Meta<typeof RetentionOwnerRunFacts>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Committed: Story = {
  args: {
    details: {
      ownershipVersion: 2,
      ownerScope: "pool_upstream_attempt_archive",
      dryRun: false,
      result: "complete",
      globalBacklogTotal: 12_480,
      discovered: 420,
      selected: 420,
      committed: 420,
      selectedScope: {
        scope: "pool_upstream_request_attempts:retention_policy",
        unit: "rows",
        selected: 420,
        committed: 420,
        batches: 2,
        rawFilesRemoved: 0,
        hasMore: false,
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("本任务运行事实")).toBeVisible();
    await expect(canvas.getByText("实际运行 · 结果：本轮完成")).toBeVisible();
    await expect(canvas.getByText("12,480")).toBeVisible();
    await expect(canvas.getByText("覆盖：本轮窗口未发现更多候选")).toBeVisible();
  },
};

export const Deferred: Story = {
  args: {
    details: {
      ownershipVersion: 2,
      ownerScope: "quota_snapshot_compaction",
      dryRun: true,
      result: "deferred",
      globalBacklogTotal: null,
      discovered: 1_000,
      selected: 256,
      committed: 0,
      selectedScope: {
        scope: "codex_quota_snapshots:retention_policy",
        unit: "quota snapshot rows",
        selected: 256,
        committed: 0,
        batches: 0,
        rawFilesPlanned: 3,
        hasMore: true,
      },
      waitReason: "retention_work_budget",
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("只读预演 · 结果：已延期")).toBeVisible();
    await expect(canvas.getByText("codex_quota_snapshots:retention_policy")).toBeVisible();
    await expect(canvas.getByText("quota snapshot rows")).toBeVisible();
    await expect(canvas.getByText("计划文件")).toBeVisible();
    await expect(canvas.getByText("3")).toBeVisible();
    await expect(canvas.getByText("全局积压")).toBeVisible();
    await expect(canvas.getAllByText("未知").length).toBeGreaterThan(0);
    await expect(canvas.getByText("覆盖：窗口后仍有候选")).toBeVisible();
  },
};

export const Mobile: Story = {
  ...Committed,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};

export const DeferredMobile: Story = {
  ...Deferred,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};

export const RecoveredUnknownMode: Story = {
  args: {
    details: {
      ownershipVersion: 2,
      ownerScope: "retention_archive",
      dryRun: null,
      result: "failed",
      selectedScope: {
        scope: "retention_archive",
        unit: "unknown",
        selected: 0,
        committed: 0,
        batches: 0,
        hasMore: true,
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("模式未知 · 结果：失败")).toBeVisible();
    await expect(canvas.getByText("文件")).toBeVisible();
    await expect(canvas.getAllByText("未知").length).toBeGreaterThan(0);
  },
};
