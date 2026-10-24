import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import type { ManagedTaskRunDetails } from "../../lib/api";
import { RetentionRunThroughput } from "./RetentionRunThroughput";

const batches: NonNullable<ManagedTaskRunDetails["archiveBatches"]> = [
  {
    dataset: "codex_invocations",
    monthKey: "2026-09",
    batchRows: 866,
    committedRows: 866,
    committedRowsPerSecond: 866 / 500,
    arrivalRowsPerSecond: 0.673,
    serviceRateMultiple: 866 / 500 / 0.673,
    filePrepareMs: 3600,
    sourceIdentityMs: 420,
    archiveWriterMs: 2500,
    summaryProofMs: 180,
    lockWaitMs: 125,
  },
  {
    dataset: "pool_upstream_request_attempts",
    monthKey: "2026-09",
    batchRows: 1000,
    committedRows: 1000,
    committedRowsPerSecond: 1000 / 500,
    arrivalRowsPerSecond: 0.4167,
    serviceRateMultiple: 1000 / 500 / 0.4167,
    filePrepareMs: 1800,
    sourceIdentityMs: 380,
    archiveWriterMs: 1900,
    summaryProofMs: null,
    lockWaitMs: 80,
  },
];

const upstreamFallbackBatch = {
  ...batches[1],
  // The single-batch story proves the positive execution-duration fallback.
  committedRowsPerSecond: null,
  serviceRateMultiple: null,
};

const meta = {
  title: "System/RetentionRunThroughput",
  component: RetentionRunThroughput,
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
} satisfies Meta<typeof RetentionRunThroughput>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Committed: Story = {
  args: {
    details: { timeoutCount: 0, archiveBatches: batches },
    executionDurationMs: 500_000,
    schedulerWaitMs: 1_360_000,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("调用记录 · 2026-09")).toBeVisible();
    await expect(canvas.getByText("2.6×")).toBeVisible();
    await expect(canvas.getByText("1.73 条/s")).toBeVisible();
    await expect(canvas.getByText("执行用时：500.0 s")).toBeVisible();
    await expect(canvas.getByText("调度等待：1360.0 s")).toBeVisible();
    await expect(canvas.getAllByText("SQLite 写入准入等待")).toHaveLength(2);
    await expect(canvas.getByText("0.42 s")).toBeVisible();
    await expect(canvas.getByText("2.50 s")).toBeVisible();
    await expect(canvas.getAllByText("未知")).toHaveLength(1);
    await expect(canvas.getByText("2.00 条/s")).toBeVisible();
    await expect(canvas.getByText("4.8×")).toBeVisible();
    await expect(canvas.getAllByText("1000 / 1000 条")).toHaveLength(1);
  },
};

export const MultiBatchFallbackUnknown: Story = {
  args: {
    details: {
      timeoutCount: 0,
      archiveBatches: batches.map((batch) => ({
        ...batch,
        committedRowsPerSecond: null,
        serviceRateMultiple: null,
      })),
    },
    executionDurationMs: 500_000,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("调用记录 · 2026-09")).toBeVisible();
    await expect(canvas.queryByText("1.73 条/s")).not.toBeInTheDocument();
    await expect(canvas.queryByText("2.00 条/s")).not.toBeInTheDocument();
    await expect(canvas.queryByText("2.6×")).not.toBeInTheDocument();
    await expect(canvas.queryByText("4.8×")).not.toBeInTheDocument();
  },
};

export const SingleBatchFallback: Story = {
  args: {
    details: { timeoutCount: 0, archiveBatches: [upstreamFallbackBatch] },
    executionDurationMs: 500_000,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("1000 / 1000 条")).toBeVisible();
    await expect(canvas.getByText("2.00 条/s")).toBeVisible();
    await expect(canvas.getByText("4.8×")).toBeVisible();
  },
};

export const PartialTimeout: Story = {
  args: {
    details: {
      timeoutCount: 1,
      archiveBatches: [
        {
          ...batches[0],
          committedRows: 512,
          committedRowsPerSecond: 8.53,
          serviceRateMultiple: 24.6,
        },
      ],
    },
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText(/超时兜底已触发/)).toBeVisible();
  },
};

export const Deferred: Story = {
  args: { details: { timeoutCount: 0, archiveBatches: [] } },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText("本次未开始归档批次")).toBeVisible();
  },
};
export const LegacyUnknown: Story = {
  args: { details: undefined },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText("归档吞吐：未知（本次记录未提供）")).toBeVisible();
  },
};
export const ZeroArrival: Story = {
  args: {
    details: {
      timeoutCount: 0,
      archiveBatches: [{ ...batches[0], arrivalRowsPerSecond: 0, serviceRateMultiple: null }],
    },
  },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).getByText("0.00 条/s")).toBeVisible();
  },
};
export const Mobile: Story = {
  ...Committed,
  globals: { viewport: { value: "mobile393", isRotated: false } },
};
