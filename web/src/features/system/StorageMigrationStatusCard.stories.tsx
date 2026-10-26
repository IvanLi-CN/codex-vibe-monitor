import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { expect, fn, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { StorageMigrationResponse } from "../../lib/api";
import { ThemeProvider } from "../../theme";
import StorageMigrationStatusCard from "./StorageMigrationStatusCard";

const baseStatus: StorageMigrationResponse = {
  revision: 7,
  migrationId: "storage-migration",
  active: true,
  module: "traffic_archive",
  state: "copying",
  source: { backend: "sqlite", configured: true, identity: "sqlite:local" },
  target: { backend: "postgres", configured: true, identity: "postgres:configured" },
  authority: { backend: "sqlite", generation: 1 },
  queuedModules: ["analytics"],
  completedModules: [],
  heartbeat: { at: "2026-10-26T08:01:12Z", healthy: true },
  progress: {
    committedRows: 820_000,
    committedBytes: 1_900_000_000,
    sourceRows: 1_000_000,
    sourceBytes: 2_400_000_000,
    scanCursor: "archive_events:820000",
    capturedThrough: 890,
    appliedThrough: 887,
    pendingKeys: 3_400,
    oldestChangeAgeMs: 11_000,
    replayThroughputPerSec: 2_100,
    incomingRatePerSec: 1_500,
    converged: true,
    etaMs: 90_000,
    phase: "copying",
  },
  tables: [
    {
      tableName: "archive_events",
      state: "copying",
      committedRows: 820_000,
      committedBytes: 1_900_000_000,
      sourceRows: 1_000_000,
      sourceBytes: 2_400_000_000,
      scanCursor: "archive_events:820000",
      scanBound: 1_000_000,
      indexPreparation: { prepared: 2, total: 3, lastError: null },
      capturedThrough: 890,
      appliedThrough: 887,
      dirtyChunks: 12,
      verifiedChunks: 98,
    },
  ],
  modules: [
    {
      module: "traffic_archive",
      state: "copying",
      dependencyReady: true,
      unappliedChanges: 3_400,
      finalProofCoverage: 96.5,
      taskHold: null,
      drainState: "not_started",
      switchAttemptCount: 0,
      switchDurationMs: null,
      switchResult: null,
      failureReason: null,
    },
  ],
  backlog: { sourceP1: 3_400, pendingKeys: 3_400, oldestChangeAgeMs: 11_000 },
  verification: { dirtyChunks: 12, verifiedChunks: 98, lastError: null },
  taskHolds: {
    held: false,
    preservedTaskKeys: [],
    reason: null,
    drainState: "not_started",
    lastSafeCheckpoint: "archive_events:820000",
  },
  pressure: {
    sourceBacklog: 3_400,
    lockWaitMs: 12,
    copyThroughputPerSec: 2_100,
    incomingChangeRatePerSec: 1_500,
    diskHeadroomBytes: 18_000_000_000,
    walHeadroomBytes: null,
    freshnessAgeMs: 1_500,
    nonConvergent: false,
  },
  switch: {
    budgetMs: 3_000,
    attemptCount: 0,
    lastAttemptAt: null,
    durationMs: null,
    result: null,
    failureReason: null,
  },
  recovery: {
    required: false,
    reason: null,
    authorityBackend: "sqlite",
    authorityGeneration: 1,
  },
  controls: {
    permission: "admin",
    canStart: false,
    canPause: true,
    canResume: false,
    lastAction: "start",
    lastActionAt: "2026-10-26T08:00:00Z",
  },
  updatedAt: "2026-10-26T08:01:12Z",
};

function withEvidenceSurface(Story: () => ReactNode) {
  return (
    <I18nProvider initialLocale="zh" persistLocale={false}>
      <ThemeProvider>
        <div
          className="min-h-screen bg-base-200 p-7"
          data-visual-evidence-surface="storage-migration-status"
        >
          <div className="mx-auto max-w-7xl" data-visual-evidence-target>
            <Story />
          </div>
        </div>
      </ThemeProvider>
    </I18nProvider>
  );
}

const meta = {
  title: "System/StorageMigrationStatusCard",
  component: StorageMigrationStatusCard,
  tags: ["autodocs"],
  parameters: { layout: "padded", viewport: { defaultViewport: "desktop1660" } },
  decorators: [withEvidenceSurface],
  args: {
    status: baseStatus,
    error: null,
    isActionPending: false,
    onAction: fn(),
  },
} satisfies Meta<typeof StorageMigrationStatusCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Copying: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("storage-migration-status-card")).toHaveTextContent("复制中");
    await expect(canvas.getByText("表级 checkpoint")).toBeVisible();
  },
};

export const StateGalleryStory: Story = {
  render: () => {
    const states: StorageMigrationResponse[] = [
      {
        ...baseStatus,
        state: "pending",
        active: false,
        target: { ...baseStatus.target, configured: false, identity: "postgres:unconfigured" },
        controls: { ...baseStatus.controls, canStart: false, canPause: false },
      },
      { ...baseStatus, state: "copying" },
      {
        ...baseStatus,
        state: "catching_up",
        modules: baseStatus.modules.map((module) => ({ ...module, state: "catching_up" })),
        progress: {
          ...baseStatus.progress,
          phase: "catching_up",
          converged: false,
          etaMs: null,
          replayThroughputPerSec: 12,
          incomingRatePerSec: 14,
        },
        pressure: {
          ...baseStatus.pressure,
          nonConvergent: true,
          copyThroughputPerSec: 12,
          incomingChangeRatePerSec: 14,
        },
      },
      {
        ...baseStatus,
        state: "paused",
        modules: baseStatus.modules.map((module) => ({ ...module, state: "paused" })),
        taskHolds: {
          held: true,
          preservedTaskKeys: ["archive-rollup"],
          reason: "operator_pause",
          drainState: "draining",
          lastSafeCheckpoint: "archive_events:820000",
        },
        controls: { ...baseStatus.controls, canPause: false, canResume: true },
      },
      {
        ...baseStatus,
        state: "recovering",
        modules: baseStatus.modules.map((module) => ({ ...module, state: "recovering" })),
        recovery: {
          required: true,
          reason: "switch outcome is indeterminate",
          authorityBackend: "sqlite",
          authorityGeneration: 1,
        },
        controls: { ...baseStatus.controls, canPause: false },
      },
    ];
    return (
      <div className="grid gap-6">
        {states.map((status) => (
          <StorageMigrationStatusCard
            key={`${status.state}-${status.target.configured}`}
            status={status}
          />
        ))}
      </div>
    );
  },
  name: "状态总览",
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByTestId("storage-migration-status-card")).toHaveLength(5);
    await expect(canvas.getByText("追赶尚未收敛")).toBeVisible();
    await expect(canvas.getByText("需要恢复")).toBeVisible();
  },
};

export const CatchingUpWithoutEta: Story = {
  args: {
    status: {
      ...baseStatus,
      state: "catching_up",
      modules: baseStatus.modules.map((module) => ({ ...module, state: "catching_up" })),
      progress: {
        ...baseStatus.progress,
        phase: "catching_up",
        converged: false,
        etaMs: null,
        replayThroughputPerSec: 12,
        incomingRatePerSec: 14,
      },
      pressure: { ...baseStatus.pressure, nonConvergent: true },
    },
  },
};

export const RecoveryRequired: Story = {
  args: {
    status: {
      ...baseStatus,
      state: "recovering",
      modules: baseStatus.modules.map((module) => ({ ...module, state: "recovering" })),
      recovery: {
        required: true,
        reason: "switch outcome is indeterminate",
        authorityBackend: "sqlite",
        authorityGeneration: 1,
      },
      controls: { ...baseStatus.controls, canPause: false },
    },
  },
};
