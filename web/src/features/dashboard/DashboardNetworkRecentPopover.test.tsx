/** @vitest-environment jsdom */
import type { ReactNode } from "react";
import { act, forwardRef, useEffect, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DashboardRecentNetworkWindowResponse } from "../../lib/api";
import {
  DashboardNetworkRecentPanel,
  DashboardNetworkRecentPopover,
} from "./DashboardNetworkRecentPopover";

const hookState = vi.hoisted(() => ({
  data: null as DashboardRecentNetworkWindowResponse | null,
  isLoading: false,
  isStale: false,
  error: null as string | null,
}));

const viewportState = vi.hoisted(() => ({
  compact: false,
}));

const overlayState = vi.hoisted(() => ({
  popoverOpen: false,
  dialogOpen: false,
}));

vi.mock("../../hooks/useCompactViewport", () => ({
  useCompactViewport: () => viewportState.compact,
}));

vi.mock("../../hooks/useDashboardRecentNetworkWindow", () => ({
  useDashboardRecentNetworkWindow: () => ({
    data: hookState.data,
    isLoading: hookState.isLoading,
    isRefreshing: false,
    isStale: hookState.isStale,
    error: hookState.error,
    reload: vi.fn(),
  }),
}));

vi.mock("../../theme", () => ({
  useTheme: () => ({
    themeMode: "dark",
  }),
}));

vi.mock("../../components/ui/popover", () => ({
  Popover: ({
    open,
    children,
    onOpenChange,
  }: {
    open: boolean;
    children: ReactNode;
    onOpenChange?: (open: boolean) => void;
  }) => {
    const rootRef = useRef<HTMLDivElement | null>(null);
    overlayState.popoverOpen = open;
    useEffect(() => {
      if (!open) {
        return;
      }
      const handleKeyDown = (event: KeyboardEvent) => {
        if (event.key === "Escape") {
          onOpenChange?.(false);
        }
      };
      const handlePointerDown = (event: PointerEvent) => {
        if (event.target instanceof Node && !rootRef.current?.contains(event.target)) {
          onOpenChange?.(false);
        }
      };
      document.addEventListener("keydown", handleKeyDown);
      document.addEventListener("pointerdown", handlePointerDown);
      return () => {
        document.removeEventListener("keydown", handleKeyDown);
        document.removeEventListener("pointerdown", handlePointerDown);
      };
    }, [onOpenChange, open]);
    return (
      <div ref={rootRef} data-open={open}>
        {children}
      </div>
    );
  },
  PopoverTrigger: ({ children }: { children: ReactNode; asChild?: boolean }) => children,
}));

vi.mock("../../components/ui/bubble-popover", () => ({
  BubblePopoverContent: forwardRef<
    HTMLDivElement,
    React.HTMLAttributes<HTMLDivElement> & { children: ReactNode }
  >(({ children, ...props }, ref) =>
    overlayState.popoverOpen ? (
      <div ref={ref} role="dialog" {...props}>
        {children}
      </div>
    ) : null,
  ),
}));

vi.mock("../../components/ui/dialog", () => ({
  Dialog: ({
    open,
    children,
  }: {
    open: boolean;
    children: ReactNode;
    onOpenChange?: (open: boolean) => void;
  }) => {
    overlayState.dialogOpen = open;
    return <div data-open={open}>{children}</div>;
  },
  DialogContent: ({ children, ...props }: { children: ReactNode }) =>
    overlayState.dialogOpen ? (
      <div role="dialog" {...props}>
        {children}
      </div>
    ) : null,
  DialogDescription: ({ children, ...props }: { children: ReactNode }) => (
    <div {...props}>{children}</div>
  ),
  DialogTitle: ({ children, ...props }: { children: ReactNode }) => (
    <div {...props}>{children}</div>
  ),
  DialogCloseIcon: ({ ...props }: { "aria-label"?: string }) => <button type="button" {...props} />,
}));

vi.mock("../../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string, vars?: Record<string, string>) => {
      const map: Record<string, string> = {
        "dashboard.networkRecent.title": "最近 5 分钟网速",
        "dashboard.networkRecent.subtitle": "查看全局最近 5 分钟的上传/下载逐秒变化",
        "dashboard.networkRecent.windowRange": `${vars?.start ?? ""} - ${vars?.end ?? ""}`,
        "dashboard.networkRecent.scope": "全局口径",
        "dashboard.networkRecent.loading": "正在加载最近网速历史",
        "dashboard.networkRecent.staleLoading": "正在等待网速推送同步",
        "dashboard.networkRecent.empty": "最近网速历史暂时不可用。",
        "dashboard.networkRecent.openPanel": "打开最近网速诊断面板",
        "dashboard.networkRecent.close": "关闭最近网速诊断面板",
        "dashboard.activityOverview.networkUpload": "上行",
        "dashboard.activityOverview.networkDownload": "下行",
        "dashboard.activityOverview.networkRefreshing": "刷新中",
      };
      return map[key] ?? key;
    },
  }),
}));

vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div data-testid="responsive">{children}</div>
  ),
  AreaChart: () => <div data-testid="area-chart" />,
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: () => <div data-testid="tooltip" />,
  Area: () => <div data-testid="area" />,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function createResponse(overrides: Partial<DashboardRecentNetworkWindowResponse> = {}) {
  return {
    rangeStart: "2026-07-20T10:00:00.000Z",
    rangeEnd: "2026-07-20T10:05:00.000Z",
    windowSeconds: 300,
    sampleSeconds: 1,
    isWarmingUp: false,
    points: [
      {
        sampleStart: "2026-07-20T10:04:59.000Z",
        sampleEnd: "2026-07-20T10:05:00.000Z",
        uploadBytesPerSecond: 3_072,
        downloadBytesPerSecond: 12_288,
        uploadBytes: 3_072,
        downloadBytes: 12_288,
        isAvailable: true,
      },
    ],
    ...overrides,
  } satisfies DashboardRecentNetworkWindowResponse;
}

beforeEach(() => {
  vi.useFakeTimers();
  viewportState.compact = false;
  overlayState.popoverOpen = false;
  overlayState.dialogOpen = false;
  hookState.data = createResponse();
  hookState.isLoading = false;
  hookState.isStale = false;
  hookState.error = null;
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  root = null;
  host?.remove();
  host = null;
  vi.useRealTimers();
});

function render(ui: ReactNode) {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

describe("DashboardNetworkRecentPopover", () => {
  it("opens on hover, locks on click, and closes on escape", async () => {
    render(
      <DashboardNetworkRecentPopover
        triggerAriaLabel="打开最近网速诊断面板"
        trigger={<span>Trigger</span>}
      />,
    );

    const trigger = host?.querySelector('[data-testid="dashboard-network-recent-trigger"]');
    expect(trigger).toBeInstanceOf(HTMLButtonElement);

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).not.toBeNull();

    await act(async () => {
      trigger?.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(
        new PointerEvent("pointerleave", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: null }));
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).toBeNull();

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).not.toBeNull();

    await act(async () => {
      trigger?.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(
        new PointerEvent("pointerleave", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: null }));
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).not.toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).toBeNull();
  });

  it("keeps the recent panel open during a slow pointer crossing into its content", async () => {
    render(
      <DashboardNetworkRecentPopover
        triggerAriaLabel="打开最近网速诊断面板"
        trigger={<span>Trigger</span>}
      />,
    );

    const trigger = host?.querySelector('[data-testid="dashboard-network-recent-trigger"]');
    expect(trigger).toBeInstanceOf(HTMLButtonElement);
    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      await Promise.resolve();
    });

    const panel = document.body.querySelector('[data-testid="dashboard-network-recent-popover"]');
    expect(panel).not.toBeNull();

    await act(async () => {
      trigger?.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(
        new PointerEvent("pointerleave", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: null }));
      vi.advanceTimersByTime(300);
      await Promise.resolve();
    });
    expect(document.body.querySelector('[data-testid="dashboard-network-recent-popover"]')).toBe(
      panel,
    );

    await act(async () => {
      panel?.dispatchEvent(
        new PointerEvent("pointerover", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      panel?.dispatchEvent(
        new PointerEvent("pointerenter", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      panel?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });
    expect(document.body.querySelector('[data-testid="dashboard-network-recent-popover"]')).toBe(
      panel,
    );
  });

  it("clears the locked state after an outside press closes the popover", async () => {
    render(
      <DashboardNetworkRecentPopover
        triggerAriaLabel="打开最近网速诊断面板"
        trigger={<span>Trigger</span>}
      />,
    );

    const trigger = host?.querySelector('[data-testid="dashboard-network-recent-trigger"]');
    expect(trigger).toBeInstanceOf(HTMLButtonElement);
    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    const outsideButton = document.createElement("button");
    document.body.appendChild(outsideButton);
    await act(async () => {
      outsideButton.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).toBeNull();

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      trigger?.dispatchEvent(
        new PointerEvent("pointerout", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      trigger?.dispatchEvent(
        new PointerEvent("pointerleave", { bubbles: true, clientX: 0, clientY: 0 }),
      );
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });
    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-popover"]'),
    ).toBeNull();
    outsideButton.remove();
  });

  it("opens the compact dialog on small viewports", async () => {
    viewportState.compact = true;

    render(
      <DashboardNetworkRecentPopover
        triggerAriaLabel="打开最近网速诊断面板"
        trigger={<span>Trigger</span>}
      />,
    );

    const trigger = host?.querySelector('[data-testid="dashboard-network-recent-trigger"]');
    expect(trigger).toBeInstanceOf(HTMLButtonElement);

    await act(async () => {
      trigger?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(
      document.body.querySelector('[data-testid="dashboard-network-recent-dialog"]'),
    ).not.toBeNull();
    expect(document.body.textContent).toContain("最近 5 分钟网速");
  });
});

describe("DashboardNetworkRecentPanel", () => {
  it("does not change hook order when loading resolves into chart data", () => {
    render(<DashboardNetworkRecentPanel response={null} loading={true} error={null} />);

    expect(() => {
      act(() => {
        root?.render(
          <DashboardNetworkRecentPanel response={createResponse()} loading={false} error={null} />,
        );
      });
    }).not.toThrow();

    expect(host?.querySelector('[data-testid="dashboard-network-recent-chart"]')).not.toBeNull();
  });

  it("keeps unavailable leading history as chart gaps without a warming prompt", () => {
    render(
      <DashboardNetworkRecentPanel
        response={createResponse({
          isWarmingUp: true,
          points: [
            {
              sampleStart: "2026-07-20T10:00:00.000Z",
              sampleEnd: "2026-07-20T10:00:01.000Z",
              uploadBytesPerSecond: 0,
              downloadBytesPerSecond: 0,
              uploadBytes: 0,
              downloadBytes: 0,
              isAvailable: false,
            },
            ...createResponse().points,
          ],
        })}
        loading={false}
        error={null}
      />,
    );

    expect(host?.querySelector('[data-testid="dashboard-network-recent-warming"]')).toBeNull();
    expect(host?.querySelector('[data-testid="dashboard-network-recent-chart"]')).not.toBeNull();
    expect(host?.textContent).not.toContain("正在积累 5 分钟历史");
  });

  it("shows the current upload and download summary without the refreshing label", () => {
    render(<DashboardNetworkRecentPanel response={createResponse()} loading={true} error={null} />);

    const summary = host?.querySelector('[data-testid="dashboard-network-recent-current-speed"]');
    expect(summary).not.toBeNull();
    expect(summary?.textContent).toContain("上行：3 KiB/s");
    expect(summary?.textContent).toContain("下行：12 KiB/s");
    expect(host?.textContent).not.toContain("刷新中");
  });

  it("covers the chart with a pushed-data loading overlay when stale", () => {
    render(
      <DashboardNetworkRecentPanel
        response={createResponse()}
        loading={false}
        stale={true}
        error={null}
      />,
    );

    const overlay = host?.querySelector('[data-testid="dashboard-network-recent-stale-overlay"]');
    expect(overlay).not.toBeNull();
    expect(overlay?.textContent).toContain("正在等待网速推送同步");
    expect(host?.textContent).not.toContain("刷新中");
  });
});
