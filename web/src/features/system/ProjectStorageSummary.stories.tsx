import type { Meta, StoryObj } from "@storybook/react-vite";
import type { ReactNode } from "react";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { SystemStorageResponse } from "../../lib/api";
import ProjectStorageSummary from "../../pages/system/ProjectStorageSummary";
import { ThemeProvider } from "../../theme";

const READY_STORAGE: SystemStorageResponse = {
  totalBytes: 105 * 1024 ** 3,
  sampledAt: "2026-10-03T07:20:00Z",
  state: "ready",
  scanInProgress: false,
  stale: false,
  reason: null,
};

function StateGallery() {
  const states: Array<{
    title: string;
    storage: SystemStorageResponse;
    error?: string | null;
  }> = [
    { title: "已测量", storage: READY_STORAGE },
    {
      title: "首次扫描中",
      storage: {
        totalBytes: null,
        sampledAt: null,
        state: "preparing",
        scanInProgress: true,
        stale: false,
        reason: null,
      },
    },
    {
      title: "资源延后，保留最近读数",
      storage: { ...READY_STORAGE, state: "deferred", stale: true, reason: "resource_limit" },
    },
    {
      title: "扫描失败，保留最近读数",
      storage: { ...READY_STORAGE, state: "error", stale: true, reason: "permission_denied" },
    },
    {
      title: "首次采样未知",
      storage: {
        totalBytes: null,
        sampledAt: null,
        state: "unknown",
        scanInProgress: false,
        stale: false,
        reason: null,
      },
    },
    {
      title: "请求失败，保留客户端读数",
      storage: READY_STORAGE,
      error: "Request timed out after 10 seconds",
    },
  ];

  return (
    <div className="grid min-w-0 gap-4 lg:grid-cols-3">
      {states.map(({ title, storage, error }) => (
        <section className="min-w-0 space-y-2" key={title}>
          <h2 className="text-sm font-semibold text-base-content">{title}</h2>
          <ProjectStorageSummary
            storage={storage}
            isLoading={false}
            isRefreshing={false}
            error={error ?? null}
          />
        </section>
      ))}
    </div>
  );
}

function withEvidenceSurface(Story: () => ReactNode) {
  return (
    <I18nProvider initialLocale="zh" persistLocale={false}>
      <ThemeProvider>
        <div
          className="min-h-screen bg-base-200 p-6 text-base-content"
          data-visual-evidence-surface="project-storage-summary"
        >
          <div className="mx-auto max-w-6xl" data-visual-evidence-target>
            <Story />
          </div>
        </div>
      </ThemeProvider>
    </I18nProvider>
  );
}

const meta = {
  title: "System/ProjectStorageSummary",
  component: ProjectStorageSummary,
  tags: ["autodocs"],
  parameters: { layout: "padded", viewport: { defaultViewport: "desktop1660" } },
  decorators: [withEvidenceSurface],
  args: {
    storage: READY_STORAGE,
    isLoading: false,
    isRefreshing: false,
    error: null,
  },
} satisfies Meta<typeof ProjectStorageSummary>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Ready: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("system-storage-summary")).toHaveTextContent("105 GiB");
  },
};

export const StateGalleryStory: Story = {
  render: () => <StateGallery />,
  name: "状态总览",
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByTestId("system-storage-summary")).toHaveLength(6);
    await expect(canvas.getAllByText("105 GiB")).toHaveLength(4);
  },
};

export const Preparing: Story = {
  tags: ["test"],
  args: {
    storage: {
      totalBytes: null,
      sampledAt: null,
      state: "preparing",
      scanInProgress: true,
      stale: false,
      reason: null,
    },
    isLoading: false,
  },
};

export const DeferredWithLastGood: Story = {
  tags: ["test"],
  args: {
    storage: {
      ...READY_STORAGE,
      state: "deferred",
      stale: true,
      reason: "resource_limit",
    },
  },
};

export const ErrorRetainsLastGood: Story = {
  tags: ["test"],
  args: {
    storage: {
      ...READY_STORAGE,
      state: "error",
      stale: true,
      reason: "permission_denied",
    },
  },
};

export const FirstSampleUnknown: Story = {
  tags: ["test"],
  args: {
    storage: {
      totalBytes: null,
      sampledAt: null,
      state: "unknown",
      scanInProgress: false,
      stale: false,
      reason: null,
    },
    isLoading: false,
  },
};

export const RequestFailedRetainsValue: Story = {
  tags: ["test"],
  args: { error: "Request timed out after 10 seconds" },
};
