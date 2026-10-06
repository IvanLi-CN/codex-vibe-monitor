import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import type { ManagedTaskRunDetails } from "../../lib/api";
import { RetentionRunThroughput } from "./RetentionRunThroughput";

const batches: NonNullable<ManagedTaskRunDetails["archiveBatches"]> = [
  {
    dataset: "codex_invocations",
    monthKey: "2026-09",
    batchRows: 1000,
    committedRows: 1000,
    committedRowsPerSecond: 32.26,
    arrivalRowsPerSecond: 0.3472,
    serviceRateMultiple: 92.9,
    filePrepareMs: 3600,
    lockWaitMs: 125,
  },
  {
    dataset: "pool_upstream_request_attempts",
    monthKey: "2026-09",
    batchRows: 1000,
    committedRows: 1000,
    committedRowsPerSecond: 32.26,
    arrivalRowsPerSecond: 0.4167,
    serviceRateMultiple: 77.4,
    filePrepareMs: 1800,
    lockWaitMs: 80,
  },
];

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
  args: { details: { timeoutCount: 0, archiveBatches: batches } },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("调用记录 · 2026-09")).toBeVisible();
    await expect(canvas.getByText("92.9×")).toBeVisible();
    await expect(canvas.getAllByText("1000 / 1000 条")).toHaveLength(2);
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
