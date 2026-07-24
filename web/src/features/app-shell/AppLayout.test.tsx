/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { AppLayout, HEADER_BRAND_ACTIVITY_HOLD_MS } from "./AppLayout";

const sseMocks = vi.hoisted(() => {
  const state = {
    lastMessageListener: null as ((payload?: unknown) => void) | null,
    subscribeToSseActivity: vi.fn((listener: (payload?: unknown) => void) => {
      state.lastMessageListener = listener;
      return () => {
        if (state.lastMessageListener === listener) {
          state.lastMessageListener = null;
        }
      };
    }),
    requestImmediateReconnect: vi.fn(),
  };
  return state;
});

const hookMocks = vi.hoisted(() => ({
  useAppVersion: vi.fn(() => ({
    versionInfo: { backend: "v0.2.0" },
    isLoading: false,
    error: null,
    refresh: vi.fn(),
  })),
  useSseDiagnostics: vi.fn(() => ({
    attempt: 3,
    reason: "manual",
    activeTopics: ["dashboard.activity.current", "stats.summary.current"],
    resumeTopics: [],
    forcedSnapshotTopics: ["dashboard.activity.current", "stats.summary.current"],
    lastMessageAt: null,
    lastOpenAt: null,
    lastErrorAt: null,
    lastConnectionStartedAt: null,
    lastTerminalOutcome: "eventsource-error",
  })),
  useSseStatus: vi.fn(() => ({
    phase: "connected",
    downtimeMs: 0,
    autoReconnect: true,
    nextRetryAt: null,
  })),
  useUpdateAvailable: vi.fn(() => ({
    currentVersion: null,
    availableVersion: null,
    visible: false,
    dismiss: vi.fn(),
    reload: vi.fn(),
  })),
  usePwaRuntime: vi.fn(() => ({
    installMode: "unsupported",
    installSupported: false,
    isOffline: false,
    shellReady: false,
    update: {
      currentVersion: "v0.2.0",
      availableVersion: null,
      visible: false,
    },
    promptInstall: vi.fn(),
    applyUpdate: vi.fn(),
    dismissUpdate: vi.fn(),
  })),
}));

vi.mock("../../lib/sse", () => ({
  subscribeToSseActivity: sseMocks.subscribeToSseActivity,
  requestImmediateReconnect: sseMocks.requestImmediateReconnect,
}));

vi.mock("../../hooks/useSseStatus", () => ({
  default: hookMocks.useSseStatus,
}));

vi.mock("../../hooks/useSseDiagnostics", () => ({
  default: hookMocks.useSseDiagnostics,
}));

vi.mock("../../hooks/useAppVersion", () => ({
  useAppVersion: hookMocks.useAppVersion,
}));

vi.mock("../../hooks/useUpdateAvailable", () => ({
  default: hookMocks.useUpdateAvailable,
}));

vi.mock("../../hooks/usePwaRuntime", () => ({
  default: hookMocks.usePwaRuntime,
}));

vi.mock("../../i18n", () => ({
  supportedLocales: ["zh", "en"],
  useTranslation: () => ({
    locale: "zh",
    setLocale: vi.fn(),
    t: (key: string, values?: Record<string, string | number>) => {
      switch (key) {
        case "app.nav.dashboard":
          return "总览";
        case "app.nav.stats":
          return "统计";
        case "app.nav.live":
          return "实况";
        case "app.nav.records":
          return "记录";
        case "app.nav.accountPool":
          return "号池";
        case "app.nav.system":
          return "系统";
        case "app.brand":
          return "Codex Vibe Monitor";
        case "app.logoAlt":
          return "product icon";
        case "app.theme.currentDark":
          return "深色";
        case "app.theme.currentLight":
          return "浅色";
        case "app.theme.switchToLight":
          return "切换浅色";
        case "app.theme.switchToDark":
          return "切换深色";
        case "app.theme.switcherAria":
          return "切换主题";
        case "app.language.option.zh":
          return "中文";
        case "app.language.option.en":
          return "English";
        case "app.language.switcherAria":
          return "切换语言";
        case "app.pwa.install.promptButton":
          return "安装应用";
        case "app.pwa.install.laterButton":
          return "稍后";
        case "app.pwa.install.manualButton":
          return "添加到主屏幕";
        case "app.pwa.install.installedButton":
          return "已安装";
        case "app.pwa.install.switcherAria":
          return "打开安装应用入口";
        case "app.pwa.install.close":
          return "关闭";
        case "app.pwa.install.closeAria":
          return "关闭安装说明";
        case "app.pwa.install.shellReady":
          return "离线壳已就绪";
        case "app.pwa.install.shellPending":
          return "离线壳待完成";
        case "app.pwa.install.offlineChip":
          return "当前离线";
        case "app.pwa.install.promptTitle":
          return "安装 Codex Vibe Monitor";
        case "app.pwa.install.promptDescription":
          return "安装为独立应用窗口";
        case "app.pwa.install.promptHint":
          return "离线时仍可打开壳层";
        case "app.pwa.install.manualTitle":
          return "添加到主屏幕";
        case "app.pwa.install.manualDescription":
          return "Safari 手动添加";
        case "app.pwa.install.manualStepOpenShare":
          return "打开分享菜单";
        case "app.pwa.install.manualStepAdd":
          return "选择添加到主屏幕";
        case "app.pwa.install.manualStepConfirm":
          return "确认图标名称";
        case "app.pwa.install.installedTitle":
          return "应用已安装";
        case "app.pwa.install.installedDescription":
          return "已运行在独立壳层";
        case "app.pwa.install.installedHint":
          return "壳层可离线打开";
        case "app.pwa.offline.title":
          return "离线应用壳层";
        case "app.pwa.offline.descriptionReady":
          return "缓存壳层仍可继续打开";
        case "app.pwa.offline.descriptionPending":
          return "请先在线访问一次";
        case "app.pwa.update.available":
          return "新的应用壳层已就绪";
        case "app.pwa.update.refresh":
          return "更新应用";
        case "app.pwa.update.later":
          return "稍后";
        case "app.footer.newVersionAvailable":
          return "新版本可用";
        case "app.footer.frontendVersion":
          return "前端版本";
        case "app.footer.backendVersion":
          return "后端版本";
        case "app.footer.versionUnavailable":
          return "不可用";
        case "app.footer.sameVersion":
          return "已同步";
        case "app.footer.updateAvailable":
          return "可更新";
        case "app.sse.banner.durationChip":
          return `${values?.minutes ?? 0}:${values?.seconds ?? "00"}`;
        case "app.sse.banner.retryingNow":
          return "正在重连";
        case "app.sse.banner.autoDisabled":
          return "自动重连已关闭";
        case "app.sse.banner.title":
          return "连接异常";
        case "app.sse.banner.description":
          return "SSE 断开";
        case "app.sse.banner.reconnectButton":
          return "立即重连";
        case "app.sse.banner.diagnostics":
          return `Attempt ${values?.attempt ?? "-"} · ${values?.reason ?? "unknown"} · topics ${
            values?.topics ?? 0
          } · resume ${values?.resume ?? 0} · forced snapshot ${values?.fresh ?? 0} · last msg ${
            values?.lastMessageAge ?? "never"
          } · ${values?.outcome ?? "unknown"}`;
        case "app.sse.banner.diagAgeNever":
          return "never";
        case "app.sse.banner.diagAgeSeconds":
          return `${values?.seconds ?? 0}s ago`;
        case "app.sse.banner.diagAgeMinutesSeconds":
          return `${values?.minutes ?? 0}m ${values?.seconds ?? 0}s ago`;
        case "app.sse.banner.diagUnknown":
          return "unknown";
        case "app.sse.reason.initial":
          return "initial";
        case "app.sse.reason.topicChange":
          return "topic change";
        case "app.sse.reason.topicRefresh":
          return "topic refresh";
        case "app.sse.reason.manual":
          return "manual reconnect";
        case "app.sse.reason.eventsourceError":
          return "event error";
        case "app.sse.reason.watchdogClosed":
          return "connection closed";
        case "app.sse.reason.watchdogTimeout":
          return "connection timeout";
        case "app.sse.reason.visibilityVisible":
          return "tab visible";
        case "app.sse.outcome.idle":
          return "idle";
        case "app.sse.outcome.open":
          return "opened";
        case "app.sse.outcome.topicChange":
          return "replaced for topic change";
        case "app.sse.outcome.eventsourceError":
          return "event error";
        case "app.sse.outcome.watchdogClosed":
          return "closed";
        case "app.sse.outcome.watchdogTimeout":
          return "timeout";
        case "app.sse.outcome.disabled":
          return "disabled";
        case "app.sse.outcome.unsupported":
          return "unsupported";
        case "app.sse.outcome.cleanup":
          return "cleaned up";
        case "app.version.loading":
          return "加载中";
        default:
          return key;
      }
    },
  }),
}));

vi.mock("../../theme", () => ({
  useTheme: () => ({
    themeMode: "dark",
    toggleTheme: vi.fn(),
  }),
}));

vi.mock("./UpdateAvailableBanner", () => ({
  UpdateAvailableBanner: ({
    currentVersion,
    availableVersion,
  }: {
    currentVersion: string;
    availableVersion: string;
  }) => (
    <div
      data-testid="update-available-banner-mock"
      data-current-version={currentVersion}
      data-available-version={availableVersion}
    />
  ),
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  sseMocks.lastMessageListener = null;
  vi.useRealTimers();
  vi.clearAllMocks();
});

function render(initialEntry = "/dashboard") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/" element={<AppLayout />}>
            <Route path="dashboard" element={<div>dashboard page</div>} />
            <Route path="stats" element={<div>stats page</div>} />
            <Route path="live" element={<div>live page</div>} />
            <Route path="records" element={<div>records page</div>} />
            <Route path="account-pool" element={<div>account pool page</div>} />
            <Route path="system/*" element={<div>system page</div>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );
  });
}

describe("AppLayout", () => {
  it("uses the compact hamburger menu only through the mobile breakpoint", async () => {
    const promptInstall = vi.fn();
    hookMocks.useUpdateAvailable.mockReturnValue({
      currentVersion: null,
      availableVersion: null,
      visible: false,
      dismiss: vi.fn(),
      reload: vi.fn(),
    });
    hookMocks.useSseStatus.mockReturnValue({
      phase: "connected",
      downtimeMs: 0,
      autoReconnect: true,
      nextRetryAt: null,
    });
    hookMocks.useAppVersion.mockReturnValue({
      versionInfo: { backend: "v0.2.0" },
      isLoading: false,
      error: null,
      refresh: vi.fn(),
    });
    hookMocks.usePwaRuntime.mockReturnValue({
      installMode: "prompt",
      installSupported: true,
      isOffline: false,
      shellReady: true,
      update: {
        currentVersion: "v0.2.0",
        availableVersion: null,
        visible: false,
      },
      promptInstall,
      applyUpdate: vi.fn(),
      dismissUpdate: vi.fn(),
    });

    render("/dashboard");

    await act(async () => {
      await Promise.resolve();
    });

    const navGroup = host?.querySelector("nav .segmented-control");
    const desktopNavigation = navGroup?.parentElement;
    const mobileMenuButton = host?.querySelector(
      'button[aria-label="app.nav.openMenu"]',
    ) as HTMLButtonElement | null;
    const dashboardLink = host?.querySelector('a[href="/dashboard"]');
    const systemLink = host?.querySelector('a[href="/system"]');
    const logoMark = host?.querySelector('[data-testid="app-header-logo-mark"]');
    const logoImage = host?.querySelector('img[src="/brand-mark.svg"][alt="product icon"]');
    const installDialog = document.body.querySelector('[data-testid="pwa-install-dialog"]');

    expect(navGroup).not.toBeNull();
    expect(desktopNavigation?.className).toContain("hidden");
    expect(desktopNavigation?.className).toContain("desktop:block");
    expect(dashboardLink?.className).toContain("segmented-control-item");
    expect(dashboardLink?.className).toContain("segmented-control-item--active");
    expect(systemLink?.className).toContain("segmented-control-item");
    expect(systemLink?.className).not.toContain("segmented-control-item--active");
    expect(logoMark?.getAttribute("data-logo-state")).toBe("idle");
    expect(logoMark?.className).toContain("h-10");
    expect(logoMark?.className).toContain("w-10");
    expect(logoImage).not.toBeNull();
    expect(host?.querySelector('[data-testid="pwa-install-control"]')).toBeNull();
    expect(installDialog?.getAttribute("data-install-mode")).toBe("prompt");
    expect(installDialog?.textContent).toContain("安装 Codex Vibe Monitor");
    expect(installDialog?.textContent).toContain("稍后");

    expect(mobileMenuButton).not.toBeNull();
    expect(mobileMenuButton?.className).toContain("desktop:!hidden");
    act(() => {
      mobileMenuButton?.click();
    });
    expect(host?.querySelector("#app-mobile-navigation")).not.toBeNull();
    expect(host?.querySelector('a[href="/account-pool/groups"]')).not.toBeNull();
    expect(host?.querySelector('a[href="/system/tasks"]')).not.toBeNull();

    const installConfirmButton = document.body.querySelector(
      '[data-testid="pwa-install-confirm"]',
    ) as HTMLButtonElement | null;
    await act(async () => {
      installConfirmButton?.click();
      await Promise.resolve();
    });
    expect(promptInstall).toHaveBeenCalledTimes(1);
  });

  it("keeps the header logo mark active across bursty updates until the recent-activity window expires", async () => {
    vi.useFakeTimers();
    hookMocks.useUpdateAvailable.mockReturnValue({
      currentVersion: null,
      availableVersion: null,
      visible: false,
      dismiss: vi.fn(),
      reload: vi.fn(),
    });
    hookMocks.useSseStatus.mockReturnValue({
      phase: "connected",
      downtimeMs: 0,
      autoReconnect: true,
      nextRetryAt: null,
    });
    hookMocks.useAppVersion.mockReturnValue({
      versionInfo: { backend: "v0.2.0" },
      isLoading: false,
      error: null,
      refresh: vi.fn(),
    });

    render("/dashboard");

    await act(async () => {
      await Promise.resolve();
    });

    const logoMark = host?.querySelector('[data-testid="app-header-logo-mark"]');
    expect(logoMark?.getAttribute("data-logo-state")).toBe("idle");

    act(() => {
      sseMocks.lastMessageListener?.();
    });
    expect(logoMark?.getAttribute("data-logo-state")).toBe("active");

    await act(async () => {
      vi.advanceTimersByTime(HEADER_BRAND_ACTIVITY_HOLD_MS - 500);
      await Promise.resolve();
    });
    expect(logoMark?.getAttribute("data-logo-state")).toBe("active");

    act(() => {
      sseMocks.lastMessageListener?.();
    });

    await act(async () => {
      vi.advanceTimersByTime(1000);
      await Promise.resolve();
    });
    expect(logoMark?.getAttribute("data-logo-state")).toBe("active");

    await act(async () => {
      vi.advanceTimersByTime(HEADER_BRAND_ACTIVITY_HOLD_MS + 20);
      await Promise.resolve();
    });
    expect(logoMark?.getAttribute("data-logo-state")).toBe("idle");
  });

  it("prioritizes the app-shell update banner and shows an offline shell notice", async () => {
    hookMocks.useUpdateAvailable.mockReturnValue({
      currentVersion: "v0.2.0",
      availableVersion: "v0.3.0",
      visible: true,
      dismiss: vi.fn(),
      reload: vi.fn(),
    });
    hookMocks.usePwaRuntime.mockReturnValue({
      installMode: "installed",
      installSupported: true,
      isOffline: true,
      shellReady: true,
      update: {
        currentVersion: "v0.2.0",
        availableVersion: "v0.2.1",
        visible: true,
      },
      promptInstall: vi.fn(),
      applyUpdate: vi.fn(),
      dismissUpdate: vi.fn(),
    });

    render("/dashboard");

    await act(async () => {
      await Promise.resolve();
    });

    const banner = host?.querySelector('[data-testid="update-available-banner-mock"]');
    const offlineBanner = host?.querySelector('[data-testid="pwa-offline-banner"]');

    expect(offlineBanner).not.toBeNull();
    expect(banner?.getAttribute("data-current-version")).toBe("v0.2.0");
    expect(banner?.getAttribute("data-available-version")).toBe("v0.2.1");
  });

  it("renders SSE diagnostics in the offline banner and routes reconnect clicks through the manual reconnect action", async () => {
    hookMocks.useUpdateAvailable.mockReturnValue({
      currentVersion: null,
      availableVersion: null,
      visible: false,
      dismiss: vi.fn(),
      reload: vi.fn(),
    });
    hookMocks.useSseStatus.mockReturnValue({
      phase: "reconnecting",
      downtimeMs: 130_000,
      autoReconnect: true,
      nextRetryAt: null,
    });
    hookMocks.useSseDiagnostics.mockReturnValue({
      attempt: 7,
      reason: "manual",
      activeTopics: ["dashboard.activity.current", "stats.summary.current"],
      resumeTopics: [],
      forcedSnapshotTopics: ["dashboard.activity.current", "stats.summary.current"],
      lastMessageAt: null,
      lastOpenAt: null,
      lastErrorAt: Date.now() - 5_000,
      lastConnectionStartedAt: Date.now() - 3_000,
      lastTerminalOutcome: "eventsource-error",
    });

    render("/dashboard");

    await act(async () => {
      await Promise.resolve();
    });

    const diagnostics = host?.querySelector('[data-testid="app-sse-diagnostics"]');
    const downtime = host?.querySelector('[data-testid="app-sse-downtime"]');
    const mobileReconnectRow = host?.querySelector('[data-testid="app-sse-reconnect-mobile-row"]');
    expect(downtime?.textContent).toBe("2:10");
    expect(downtime?.className).not.toContain("rounded-full");
    expect(downtime?.className).not.toContain("bg-warning");
    expect(mobileReconnectRow?.className).toContain("justify-end");
    expect(mobileReconnectRow?.textContent).toContain("立即重连");
    expect(diagnostics?.textContent).toContain("Attempt 7");
    expect(diagnostics?.textContent).toContain("manual reconnect");
    expect(diagnostics?.textContent).toContain("topics 2");
    expect(diagnostics?.textContent).toContain("resume 0");
    expect(diagnostics?.textContent).toContain("forced snapshot 2");
    expect(diagnostics?.textContent).toContain("event error");

    const reconnectButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) =>
      button.textContent?.includes("立即重连"),
    );
    act(() => {
      reconnectButton?.click();
    });
    expect(sseMocks.requestImmediateReconnect).toHaveBeenCalledTimes(1);
  });
});
