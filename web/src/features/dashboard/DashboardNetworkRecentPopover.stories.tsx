import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";
import { I18nProvider, useTranslation } from "../../i18n";
import type { DashboardRecentNetworkWindowResponse } from "../../lib/api";
import { FullPageStorySurface } from "../../storybook/storybookPageHelpers";
import { DashboardNetworkRecentPanel } from "./DashboardNetworkRecentPopover";
import { DashboardNetworkSpeedCapsule } from "./DashboardNetworkSpeedCapsule";

const WINDOW_SECONDS = 300;
const RANGE_END = new Date("2026-07-20T10:05:00.000Z");

type StoryFlow = {
  id: number;
  startSecond: number;
  requestUploadBytesPerSecond: readonly number[];
  ttfbSeconds: number;
  downloadBytesPerSecond: readonly number[];
  keepaliveUploads?: readonly {
    secondOffset: number;
    bytesPerSecond: number;
  }[];
};

const STORY_FLOWS: readonly StoryFlow[] = [
  {
    id: 1,
    startSecond: 16,
    requestUploadBytesPerSecond: [1_180, 840],
    ttfbSeconds: 2,
    downloadBytesPerSecond: [
      620, 980, 1_310, 1_560, 1_450, 1_380, 1_240, 980, 760, 820, 1_140, 1_260, 1_100, 870, 620,
      380,
    ],
  },
  {
    id: 2,
    startSecond: 47,
    requestUploadBytesPerSecond: [1_460, 1_020, 640],
    ttfbSeconds: 3,
    downloadBytesPerSecond: [
      420, 760, 1_120, 1_580, 1_940, 2_260, 2_410, 2_180, 1_960, 1_680, 1_360, 1_180, 940, 1_020,
      1_280, 1_420, 1_340, 1_180, 920, 640, 420,
    ],
    keepaliveUploads: [
      { secondOffset: 6, bytesPerSecond: 36 },
      { secondOffset: 12, bytesPerSecond: 28 },
    ],
  },
  {
    id: 3,
    startSecond: 64,
    requestUploadBytesPerSecond: [920, 560],
    ttfbSeconds: 2,
    downloadBytesPerSecond: [
      360, 720, 980, 1_220, 1_140, 980, 820, 660, 540, 620, 820, 980, 860, 680, 420,
    ],
  },
  {
    id: 4,
    startSecond: 129,
    requestUploadBytesPerSecond: [1_620, 1_260, 760],
    ttfbSeconds: 2,
    downloadBytesPerSecond: [
      540, 920, 1_480, 2_140, 2_480, 2_620, 2_380, 2_100, 1_840, 1_520, 1_220, 960, 1_180, 1_380,
      1_620, 1_740, 1_680, 1_480, 1_240, 920, 680, 460,
    ],
  },
  {
    id: 5,
    startSecond: 145,
    requestUploadBytesPerSecond: [760, 520, 280],
    ttfbSeconds: 1,
    downloadBytesPerSecond: [
      480, 820, 1_160, 1_040, 920, 780, 620, 540, 700, 960, 1_120, 980, 760, 520,
    ],
  },
  {
    id: 6,
    startSecond: 221,
    requestUploadBytesPerSecond: [1_220, 960, 620],
    ttfbSeconds: 1,
    downloadBytesPerSecond: [
      620, 980, 1_460, 1_880, 2_140, 2_260, 2_120, 1_860, 1_540, 1_280, 980, 760, 880, 1_120, 1_360,
      1_480, 1_420, 1_220, 920, 620, 420,
    ],
    keepaliveUploads: [{ secondOffset: 7, bytesPerSecond: 20 }],
  },
  {
    id: 7,
    startSecond: 271,
    requestUploadBytesPerSecond: [640, 460],
    ttfbSeconds: 1,
    downloadBytesPerSecond: [
      420, 760, 1_040, 980, 840, 720, 620, 540, 460, 520, 680, 860, 920, 840, 720, 580, 460, 360,
      280, 220, 180, 140, 120, 160, 220, 280,
    ],
  },
] as const;

function buildStoryFlowRates(second: number) {
  let uploadBytesPerSecond = 24;
  let downloadBytesPerSecond = 160;

  for (const flow of STORY_FLOWS) {
    if (second < flow.startSecond) {
      continue;
    }

    const uploadOffset = second - flow.startSecond;
    if (uploadOffset < flow.requestUploadBytesPerSecond.length) {
      uploadBytesPerSecond += flow.requestUploadBytesPerSecond[uploadOffset] ?? 0;
    }

    const downloadStart =
      flow.startSecond + flow.requestUploadBytesPerSecond.length + flow.ttfbSeconds;
    const downloadOffset = second - downloadStart;
    if (downloadOffset >= 0 && downloadOffset < flow.downloadBytesPerSecond.length) {
      downloadBytesPerSecond += flow.downloadBytesPerSecond[downloadOffset] ?? 0;
    }

    for (const keepaliveUpload of flow.keepaliveUploads ?? []) {
      if (downloadOffset === keepaliveUpload.secondOffset) {
        uploadBytesPerSecond += keepaliveUpload.bytesPerSecond;
      }
    }
  }

  return {
    uploadBytesPerSecond,
    downloadBytesPerSecond,
  };
}

function buildSampleResponse(unavailablePrefixSeconds = 0): DashboardRecentNetworkWindowResponse {
  const rangeStart = new Date(RANGE_END.getTime() - WINDOW_SECONDS * 1000);
  const points = Array.from({ length: WINDOW_SECONDS }, (_, index) => {
    const sampleStart = new Date(rangeStart.getTime() + index * 1000);
    const sampleEnd = new Date(sampleStart.getTime() + 1000);
    const isAvailable = index >= unavailablePrefixSeconds;

    let uploadBytesPerSecond = 0;
    let downloadBytesPerSecond = 0;

    if (isAvailable) {
      const offset = index - unavailablePrefixSeconds;
      const rates = buildStoryFlowRates(offset);
      uploadBytesPerSecond = rates.uploadBytesPerSecond;
      downloadBytesPerSecond = rates.downloadBytesPerSecond;
    }

    return {
      sampleStart: sampleStart.toISOString(),
      sampleEnd: sampleEnd.toISOString(),
      uploadBytesPerSecond,
      downloadBytesPerSecond,
      uploadBytes: uploadBytesPerSecond,
      downloadBytes: downloadBytesPerSecond,
      isAvailable,
    };
  });

  return {
    rangeStart: rangeStart.toISOString(),
    rangeEnd: RANGE_END.toISOString(),
    windowSeconds: WINDOW_SECONDS,
    sampleSeconds: 1,
    isWarmingUp: unavailablePrefixSeconds > 0,
    points,
  };
}

const populatedResponse = buildSampleResponse();
const warmingResponse = buildSampleResponse(96);

function DesktopLockedPreview({ response }: { response: DashboardRecentNetworkWindowResponse }) {
  const latestAvailablePoint =
    [...response.points].reverse().find((point) => point.isAvailable) ?? null;

  return (
    <FullPageStorySurface>
      <section className="surface-panel overflow-visible">
        <div className="surface-panel-body gap-4 sm:gap-5">
          <div className="flex justify-end">
            <DashboardNetworkSpeedCapsule
              uploadBytesPerSecond={latestAvailablePoint?.uploadBytesPerSecond ?? 0}
              downloadBytesPerSecond={latestAvailablePoint?.downloadBytesPerSecond ?? 0}
              localeTag="zh-CN"
              uploadLabel="上行"
              downloadLabel="下行"
              className="bg-base-100/62"
            />
          </div>
          <div className="flex justify-end">
            <div className="w-full max-w-[52rem]">
              <DashboardNetworkRecentPanel response={response} loading={false} error={null} />
            </div>
          </div>
        </div>
      </section>
    </FullPageStorySurface>
  );
}

function CompactSheetPreview({ response }: { response: DashboardRecentNetworkWindowResponse }) {
  const { t } = useTranslation();

  return (
    <div className="min-h-screen bg-[#08172b] px-3 py-6 text-white">
      <div className="mx-auto flex min-h-[calc(100vh-3rem)] max-w-[430px] items-end">
        <div
          data-theme="vibe-dark"
          className="w-full overflow-hidden rounded-[1.75rem] border border-base-300/70 bg-base-100 shadow-[0_32px_72px_rgba(3,9,20,0.55)]"
        >
          <div className="flex items-start gap-3 border-b border-base-300/70 px-4 py-4 sm:px-5">
            <div className="min-w-0 flex-1">
              <div className="min-w-0 text-lg font-semibold">
                {t("dashboard.networkRecent.title")}
              </div>
              <div className="mt-1 text-sm leading-6 text-base-content/68">
                {t("dashboard.networkRecent.subtitle")}
              </div>
            </div>
            <button
              type="button"
              aria-label={t("dashboard.networkRecent.close")}
              className="inline-flex h-9 w-9 items-center justify-center rounded-full border border-base-300/70 bg-base-200/80 text-xl leading-none text-base-content/72"
            >
              ×
            </button>
          </div>
          <div className="max-h-[min(100dvh-7rem,48rem)] overflow-y-auto px-4 py-4 sm:px-5">
            <DashboardNetworkRecentPanel response={response} loading={false} error={null} />
          </div>
        </div>
      </div>
    </div>
  );
}

const meta = {
  title: "Dashboard/DashboardNetworkRecentPopover",
  component: DashboardNetworkRecentPanel,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  globals: {
    themeMode: "dark",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <Story />
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof DashboardNetworkRecentPanel>;

export default meta;

type Story = StoryObj<typeof meta>;

export const DesktopFixedOpen: Story = {
  args: {
    response: null,
    loading: false,
    error: null,
  },
  render: () => <DesktopLockedPreview response={populatedResponse} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    const panel = canvasElement.querySelector('[data-testid="dashboard-network-recent-panel"]');
    expect(panel).not.toBeNull();
    expect(canvasElement.textContent ?? "").toContain("最近 5 分钟网速");
  },
};

export const DesktopFixedOpenLight: Story = {
  args: {
    response: null,
    loading: false,
    error: null,
  },
  render: () => <DesktopLockedPreview response={populatedResponse} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  globals: {
    themeMode: "light",
  },
  play: async ({ canvasElement }) => {
    const panel = canvasElement.querySelector('[data-testid="dashboard-network-recent-panel"]');
    expect(panel).not.toBeNull();
    expect(canvasElement.textContent ?? "").toContain("最近 5 分钟网速");
  },
};

export const CompactSheetWarming: Story = {
  args: {
    response: null,
    loading: false,
    error: null,
  },
  render: () => <CompactSheetPreview response={warmingResponse} />,
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const warming = canvasElement.querySelector('[data-testid="dashboard-network-recent-warming"]');
    expect(warming).not.toBeNull();
    expect(canvasElement.textContent ?? "").toContain("最近 5 分钟网速");
  },
};
