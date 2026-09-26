import type { Meta, StoryObj } from "@storybook/react-vite";
import { act, useRef, useState } from "react";
import { expect, userEvent, within } from "storybook/test";
import { BubblePopoverContent } from "../../components/ui/bubble-popover";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "../../components/ui/dialog";
import { Popover, PopoverTrigger } from "../../components/ui/popover";
import { usePointerTransitionGuard } from "../../hooks/usePointerTransitionGuard";
import { I18nProvider, useTranslation } from "../../i18n";
import type { DashboardRecentNetworkWindowResponse } from "../../lib/api";
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
const partialHistoryResponse = buildSampleResponse(96);

function DesktopLockedPreview({
  response,
  stale = false,
  ambientClassName = "bg-[#071a31]",
}: {
  response: DashboardRecentNetworkWindowResponse;
  stale?: boolean;
  ambientClassName?: string;
}) {
  const latestAvailablePoint =
    [...response.points].reverse().find((point) => point.isAvailable) ?? null;

  return (
    <div
      data-visual-evidence-surface
      className={`inline-flex max-w-full p-[18px] text-base-content ${ambientClassName}`}
    >
      <section
        data-visual-evidence-target
        className="surface-panel w-[56rem] max-w-[calc(100vw-4rem)] overflow-visible"
      >
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
              <DashboardNetworkRecentPanel
                response={response}
                loading={false}
                stale={stale}
                error={null}
              />
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}

function CompactSheetPreview({
  response,
  stale = false,
}: {
  response: DashboardRecentNetworkWindowResponse;
  stale?: boolean;
}) {
  const { t } = useTranslation();

  return (
    <div data-visual-evidence-surface className="inline-flex bg-[#08172b] p-4 text-white">
      <div
        data-visual-evidence-target
        data-theme="vibe-dark"
        className="w-[398px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-[1.75rem] border border-base-300/70 bg-base-100 shadow-[0_32px_72px_rgba(3,9,20,0.55)]"
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
          <DashboardNetworkRecentPanel
            response={response}
            loading={false}
            stale={stale}
            error={null}
          />
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

function RecentPopoverTransferPreview() {
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(true);
  const [locked, setLocked] = useState(false);
  const pointerTransition = usePointerTransitionGuard({
    triggerRef,
    contentRef,
    onClose: () => setOpen(false),
    pinned: locked,
  });

  return (
    <div data-visual-evidence-surface className="min-h-screen bg-base-200 p-8 text-base-content">
      <div
        data-visual-evidence-target
        className="flex min-h-32 items-start justify-end rounded-xl border border-base-300/65 bg-base-100/60 p-5"
      >
        <Popover
          open={open}
          onOpenChange={(nextOpen) => {
            if (!nextOpen) {
              pointerTransition.cancel();
              setOpen(false);
              setLocked(false);
            }
          }}
        >
          <PopoverTrigger asChild>
            <button
              ref={triggerRef}
              type="button"
              aria-label="Open recent network diagnostics"
              aria-haspopup="dialog"
              aria-expanded={open}
              data-testid="dashboard-network-recent-trigger"
              onMouseEnter={() => {
                pointerTransition.cancel();
                setOpen(true);
              }}
              onPointerEnter={() => {
                pointerTransition.cancel();
                setOpen(true);
              }}
              onPointerLeave={(event) => {
                if (!locked) pointerTransition.start("trigger", event);
              }}
              onClick={(event) => {
                event.preventDefault();
                pointerTransition.cancel();
                setOpen(true);
                setLocked((current) => !current);
              }}
              className="rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
            >
              <DashboardNetworkSpeedCapsule
                uploadBytesPerSecond={3_072}
                downloadBytesPerSecond={12_288}
                localeTag="en-US"
                uploadLabel="Upload"
                downloadLabel="Download"
                testId="dashboard-upstream-account-total-network-speed"
              />
            </button>
          </PopoverTrigger>
          <BubblePopoverContent
            ref={contentRef}
            align="end"
            side="bottom"
            sideOffset={10}
            className="w-[min(52rem,calc(100vw-1rem))] max-w-[min(52rem,calc(100vw-1rem))] border-none bg-transparent p-0 shadow-none"
            onPointerEnter={() => {
              pointerTransition.cancel();
              setOpen(true);
            }}
            onPointerLeave={(event) => {
              if (!locked) pointerTransition.start("content", event);
            }}
            data-testid="dashboard-network-recent-popover"
          >
            <DashboardNetworkRecentPanel
              response={populatedResponse}
              loading={false}
              stale={false}
              error={null}
            />
          </BubblePopoverContent>
        </Popover>
      </div>
    </div>
  );
}

function CompactRecentDialogPreview() {
  const [open, setOpen] = useState(false);
  const { t } = useTranslation();

  return (
    <div data-visual-evidence-surface className="min-h-screen bg-base-200 p-4 text-base-content">
      <div data-visual-evidence-target className="flex justify-end rounded-xl bg-base-100/60 p-5">
        <button
          type="button"
          aria-label="Open recent network diagnostics"
          data-testid="dashboard-network-recent-trigger"
          onClick={() => setOpen(true)}
          className="rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <DashboardNetworkSpeedCapsule
            uploadBytesPerSecond={3_072}
            downloadBytesPerSecond={12_288}
            localeTag="en-US"
            uploadLabel="Upload"
            downloadLabel="Download"
          />
        </button>
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent
            className="max-h-[min(100dvh-0.5rem,100dvh)] overflow-hidden"
            data-testid="dashboard-network-recent-dialog"
          >
            <div className="flex items-start gap-3 border-b border-base-300/70 px-4 py-4">
              <div className="min-w-0 flex-1">
                <DialogTitle className="min-w-0 text-lg">
                  {t("dashboard.networkRecent.title")}
                </DialogTitle>
                <DialogDescription className="mt-1 text-sm leading-6 text-base-content/68">
                  {t("dashboard.networkRecent.subtitle")}
                </DialogDescription>
              </div>
              <DialogCloseIcon aria-label={t("dashboard.networkRecent.close")} />
            </div>
            <div className="max-h-[calc(min(100dvh-0.5rem,100dvh)-5.5rem)] overflow-y-auto px-4 py-4">
              <DashboardNetworkRecentPanel
                response={populatedResponse}
                loading={false}
                stale={false}
                error={null}
              />
            </div>
          </DialogContent>
        </Dialog>
      </div>
    </div>
  );
}

async function waitForTransfer(milliseconds: number) {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, milliseconds));
  });
}

export const DesktopPopoverTransfer: Story = {
  tags: ["test"],
  args: {
    response: null,
    loading: false,
    error: null,
  },
  render: () => <RecentPopoverTransferPreview />,
  parameters: {
    viewport: { defaultViewport: "desktop1440" },
  },
  play: async ({ canvasElement }) => {
    const trigger = within(canvasElement).getByTestId("dashboard-network-recent-trigger");
    const panel = await within(document.body).findByTestId("dashboard-network-recent-popover");
    const triggerRect = trigger.getBoundingClientRect();
    const panelRect = panel.getBoundingClientRect();
    const start = { x: triggerRect.right - 1, y: triggerRect.top + triggerRect.height / 2 };
    const end = {
      x: panelRect.left + panelRect.width / 2,
      y: panelRect.top + panelRect.height / 2,
    };
    await act(async () => {
      trigger.dispatchEvent(
        new PointerEvent("pointerout", {
          bubbles: true,
          pointerType: "mouse",
          clientX: start.x,
          clientY: start.y,
        }),
      );
      for (const progress of [0.45, 0.8]) {
        document.dispatchEvent(
          new PointerEvent("pointermove", {
            bubbles: true,
            pointerType: "mouse",
            clientX: start.x + (end.x - start.x) * progress,
            clientY: start.y + (end.y - start.y) * progress,
          }),
        );
        await new Promise((resolve) => window.setTimeout(resolve, 150));
      }
    });
    await waitForTransfer(300);
    await expect(within(document.body).getByTestId("dashboard-network-recent-popover")).toBe(panel);
    await act(async () => {
      panel.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, pointerType: "mouse" }));
    });
    await waitForTransfer(600);
    await expect(within(document.body).getByTestId("dashboard-network-recent-popover")).toBe(panel);
  },
};

export const CompactDialogEntry: Story = {
  ...DesktopPopoverTransfer,
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const trigger = within(canvasElement).getByTestId("dashboard-network-recent-trigger");
    await userEvent.click(trigger);
    const dialog = await within(document.body).findByTestId("dashboard-network-recent-dialog");
    await expect(dialog).toBeInTheDocument();
    await expect(within(dialog).getByTestId("dashboard-network-recent-panel")).toBeInTheDocument();
  },
  render: () => <CompactRecentDialogPreview />,
};

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
    expect(canvasElement.textContent ?? "").toContain("上行：");
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
    expect(canvasElement.textContent ?? "").toContain("下行：");
  },
};

export const DesktopStaleOverlay: Story = {
  args: {
    response: null,
    loading: false,
    error: null,
  },
  render: () => <DesktopLockedPreview response={populatedResponse} stale />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    const overlay = canvasElement.querySelector(
      '[data-testid="dashboard-network-recent-stale-overlay"]',
    );
    expect(overlay).not.toBeNull();
    expect(canvasElement.textContent ?? "").toContain("正在等待网速推送同步");
    expect(canvasElement.textContent ?? "").toContain("上行：");
  },
};

export const CompactSheetPartialHistory: Story = {
  args: {
    response: null,
    loading: false,
    error: null,
  },
  render: () => <CompactSheetPreview response={partialHistoryResponse} />,
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    expect(canvasElement.textContent ?? "").toContain("最近 5 分钟网速");
    expect(canvasElement.textContent ?? "").toContain("上行：");
    expect(canvasElement.textContent ?? "").not.toContain("正在积累 5 分钟历史");
  },
};
