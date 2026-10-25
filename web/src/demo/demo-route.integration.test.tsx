/** @vitest-environment jsdom */

import { setupServer } from "msw/node";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, useLocation } from "react-router-dom";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import App from "../App";
import { SystemNotificationProvider } from "../components/ui/system-notifications";
import { I18nProvider } from "../i18n";
import { ThemeProvider } from "../theme";
import { installDemoEventSource } from "./event-source";
import { installDemoFetchFallback } from "./fallback";
import { apiHandlers, handleDemoRequest } from "./handlers";
import { demoModel } from "./model";
import { demoRoutes, demoSemanticMatrix } from "./route-matrix";
import type { DemoScene } from "./runtime";

let host: HTMLDivElement;
let root: Root;
let cleanupFetch: () => void;
let cleanupEventSource: () => void;
let cleanupFetchAudit: () => void;
const previousMatchMedia = window.matchMedia;
const previousEventSource = window.EventSource;
const previousCss = globalThis.CSS;
const previousGetBBox = SVGElement.prototype.getBBox;
const unexpectedDemoApiResponses: string[] = [];
const unhandledRequests: string[] = [];
const failedDemoRequests: string[] = [];
const server = setupServer(...apiHandlers);

function RouteProbe() {
  const location = useLocation();
  return <output data-testid="demo-route-probe">{location.pathname}</output>;
}

function routeEntry(path: string): { pathname: string; search: string } {
  const hash = path.slice(path.indexOf("#") + 1);
  const queryIndex = hash.indexOf("?");
  return queryIndex === -1
    ? { pathname: hash, search: "" }
    : { pathname: hash.slice(0, queryIndex), search: hash.slice(queryIndex) };
}

function renderRoute(path: string, scene: DemoScene) {
  const { pathname, search } = routeEntry(path);
  window.location.hash = `#/?demoScene=${scene}&demoTheme=light`;
  demoModel.setScene(scene);
  demoModel.reset();
  act(() => {
    root.render(
      <ThemeProvider>
        <I18nProvider initialLocale="zh" persistLocale={false}>
          <SystemNotificationProvider>
            <MemoryRouter key={path} initialEntries={[{ pathname, search }]}>
              <RouteProbe />
              <App />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>
      </ThemeProvider>,
    );
  });
}

function expectRouteLandmark(route: (typeof demoRoutes)[number]) {
  if (route.path === "/#/account-pool/groups") {
    expect(
      host.querySelector('[data-testid="account-pool-groups-list"]') ??
        host.querySelector('[data-testid="account-pool-groups-empty"]') ??
        null,
      route.path,
    ).not.toBeNull();
    return;
  }
  if (route.path === "/#/system/tasks/retention_archive") {
    const bodyText = document.body.textContent ?? "";
    const hasRetentionHeading =
      host.querySelector("h1, h2, h3, h4, h5, h6")?.textContent?.includes("数据保留与归档") ===
      true;
    expect(
      hasRetentionHeading ||
        bodyText.includes("正在读取任务详情") ||
        bodyText.includes("任务观测不可用"),
      route.path,
    ).toBe(true);
    return;
  }
  if (route.landmark.kind === "testid") {
    expect(
      host.querySelector(`[data-testid="${route.landmark.value}"]`) ??
        document.body.querySelector(`[data-testid="${route.landmark.value}"]`),
      route.path,
    ).not.toBeNull();
    return;
  }
  expect(
    Array.from(host.querySelectorAll("h1, h2, h3, h4, h5, h6")).some(
      (heading) => heading.textContent?.trim() === route.landmark.value,
    ),
    route.path,
  ).toBe(true);
}

function expectedNetworkFailurePath(path: string): string {
  switch (path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      return "/events";
    case "/#/stats":
      return "/api/stats/errors";
    case "/#/live":
      return "/api/pool/model-routing-live";
    case "/#/dashboard/invocations/demo-invocation-9002":
      return "/api/invocations";
    case "/#/records":
      return "/api/invocations";
    case "/#/account-pool":
    case "/#/account-pool/upstream-accounts":
    case "/#/account-pool/transits":
    case "/#/account-pool/pool":
    case "/#/account-pool/upstream-accounts/new?mode=apiKey":
    case "/#/account-pool/upstream-accounts/new?mode=oauth":
    case "/#/account-pool/groups":
      return "/api/pool/upstream-accounts";
    case "/#/account-pool/maintenance-records":
      return "/api/pool/upstream-account-events";
    case "/#/system":
    case "/#/system/status":
      return "/api/system/status";
    case "/#/system/performance":
      return "/api/system/observability";
    case "/#/system/tasks":
      return "/api/system/managed-tasks";
    case "/#/system/tasks/retention_archive":
      return "/api/system/managed-tasks/retention_archive";
    case "/#/system/prompt-cache":
      return "/api/system/prompt-cache/materialization";
    case "/#/system/models":
    case "/#/system/proxy":
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      return "/api/settings";
    default:
      throw new Error(`Missing network failure request contract for ${path}`);
  }
}

function assertNetworkFailureState(route: (typeof demoRoutes)[number]) {
  const bodyText = document.body.textContent ?? "";
  switch (route.path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      expect(bodyText).toContain("当前无法获取调用明细");
      return;
    case "/#/dashboard/invocations/demo-invocation-9002":
      expect(
        document.body.querySelector('[data-testid="dashboard-invocation-detail-error"]'),
      ).not.toBeNull();
      return;
    case "/#/stats":
      expect(host.querySelector('[data-testid="stats-error-distribution"]')?.textContent).toMatch(
        /Request failed: 0|Failed to fetch/,
      );
      return;
    case "/#/live":
      expect(
        host.querySelector('[data-testid="model-routing-live-panel"] [role="status"]')?.textContent,
      ).toMatch(/Request failed: 0|Failed to fetch/);
      return;
    case "/#/records":
      expect(host.querySelector('[data-testid="invocation-records-table-error"]')).not.toBeNull();
      return;
    case "/#/account-pool":
    case "/#/account-pool/upstream-accounts":
    case "/#/account-pool/transits":
    case "/#/account-pool/pool":
      expect(
        host.querySelector('[data-testid="upstream-accounts-table-error"]') ??
          host.querySelector('[data-testid="upstream-accounts-grouped-error"]'),
      ).not.toBeNull();
      return;
    case "/#/account-pool/groups":
      expect(host.querySelector('[data-testid="account-pool-groups-error"]')).not.toBeNull();
      return;
    case "/#/account-pool/maintenance-records":
      expect(host.querySelector('[data-testid="maintenance-records-error"]')).not.toBeNull();
      return;
    case "/#/account-pool/upstream-accounts/new?mode=apiKey":
    case "/#/account-pool/upstream-accounts/new?mode=oauth":
      expect(host.querySelector('[role="status"]')?.textContent).toMatch(
        /Request failed: 0|Failed to fetch/,
      );
      return;
    case "/#/system":
    case "/#/system/status":
      expect(bodyText).toMatch(/加载系统状态失败：/);
      return;
    case "/#/system/performance":
      expect(bodyText).toContain("暂时无法加载观测状态。");
      return;
    case "/#/system/tasks":
      expect(bodyText).toMatch(/任务目录不可用：/);
      return;
    case "/#/system/tasks/retention_archive":
      expect(bodyText).toMatch(/任务观测不可用：/);
      return;
    case "/#/system/prompt-cache":
      expect(host.querySelector('[data-testid="system-prompt-cache-error"]')).not.toBeNull();
      return;
    case "/#/system/models":
      expect(host.querySelector('[role="alert"]')?.textContent).toMatch(
        /Request failed: 0|Failed to fetch/,
      );
      return;
    case "/#/system/proxy":
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      expect(bodyText).toMatch(/设置请求失败：/);
      return;
  }
}

function assertRouteTerminalState(route: (typeof demoRoutes)[number], scene: DemoScene) {
  if (scene === "network-failure") {
    assertNetworkFailureState(route);
    expect(failedDemoRequests).toContain(`GET ${expectedNetworkFailurePath(route.path)}`);
    return;
  }

  expect(host.querySelector('[aria-busy="true"]')).toBeNull();
  switch (route.path) {
    case "/#/":
    case "/#/dashboard":
    case "/#/not-a-route":
      expect(host.querySelector('[data-testid="dashboard-activity-range-today"]')).not.toBeNull();
      expect(host.querySelector('[data-testid="dashboard-working-conversations"]')).not.toBeNull();
      if (scene === "empty") {
        expect(host.textContent).toContain("最近 5 分钟内暂无终态调用");
        expect(
          host.querySelector('[data-testid="dashboard-working-conversations-grid"]'),
        ).toBeNull();
      } else {
        expect(
          host.querySelector('[data-testid="dashboard-working-conversations-grid"]'),
        ).not.toBeNull();
      }
      return;
    case "/#/dashboard/invocations/demo-invocation-9002":
      expect(
        scene === "empty"
          ? document.body.querySelector('[data-testid="dashboard-invocation-detail-empty"]')
          : document.body.querySelector(".invocation-detail-hero-surface"),
        `${route.path} / ${scene}: ${host.textContent?.slice(-800) ?? ""}`,
      ).not.toBeNull();
      return;
    case "/#/stats": {
      const statsHeading = Array.from(host.querySelectorAll("h1, h2, h3, h4, h5, h6")).find(
        (heading) => heading.textContent?.trim() === "统计",
      );
      expect(statsHeading).not.toBeUndefined();
      let summaryPanel: Element | null = statsHeading ?? null;
      for (let index = 0; index < 4 && summaryPanel; index += 1) {
        summaryPanel = summaryPanel.parentElement;
      }
      const metrics = Array.from(summaryPanel?.querySelectorAll(".metric-value") ?? []);
      expect(metrics.length).toBeGreaterThan(0);
      const firstMetricCell = metrics[0]?.closest(".metric-cell");
      const firstMetricValue = firstMetricCell?.querySelector(
        '[data-adaptive-metric-measure-kind="full"]',
      )?.textContent;
      if (scene === "empty") {
        expect(firstMetricValue?.trim()).toBe("0");
      } else {
        expect(firstMetricValue?.trim()).not.toBe("0");
      }
      return;
    }
    case "/#/live":
      if (scene === "empty") {
        expect(host.textContent).toContain("没有符合筛选条件的 API Key 模型路由状态。");
        expect(host.querySelector('[data-testid="model-routing-gantt"]')).toBeNull();
      } else {
        expect(host.querySelector('[data-testid="model-routing-gantt"]')).not.toBeNull();
      }
      return;
    case "/#/records":
      expect(host.querySelector('[data-testid="invocation-records-table-loading"]')).toBeNull();
      expect(host.querySelector('[data-testid="invocation-records-table-error"]')).toBeNull();
      if (scene === "empty") {
        expect(host.querySelector('[data-testid="invocation-records-table-empty"]')).not.toBeNull();
      } else {
        expect(host.querySelector('[data-testid="invocation-records-model"]')).not.toBeNull();
      }
      return;
    case "/#/account-pool/groups":
      expect(
        host.querySelector(
          scene === "empty"
            ? '[data-testid="account-pool-groups-empty"]'
            : '[data-testid="account-pool-groups-list"]',
        ),
      ).not.toBeNull();
      return;
    case "/#/account-pool":
    case "/#/account-pool/upstream-accounts":
    case "/#/account-pool/transits":
    case "/#/account-pool/pool":
      if (scene === "empty") {
        expect(
          host.querySelector('[data-testid="upstream-accounts-table-empty"]') ??
            host.querySelector('[data-testid="upstream-accounts-grouped-empty"]'),
        ).not.toBeNull();
      } else {
        expect(
          host.querySelector('[data-testid="upstream-accounts-grouped-roster"]') ??
            host.querySelector('[title="alpha@demo.invalid"]') ??
            host.querySelector('[title="Aster"]'),
        ).not.toBeNull();
      }
      return;
    case "/#/account-pool/upstream-accounts/new?mode=apiKey":
    case "/#/account-pool/upstream-accounts/new?mode=oauth":
      expect(host.querySelector(".animate-spin")).toBeNull();
      expect(host.querySelector('[role="status"]')).toBeNull();
      expect(
        host.querySelector(
          `input[name="${route.path.endsWith("mode=apiKey") ? "apiKeyDisplayName" : "oauthDisplayName"}"]`,
        ),
      ).not.toBeNull();
      return;
    case "/#/account-pool/maintenance-records":
      if (scene === "empty")
        expect(host.querySelector('[data-testid="maintenance-records-empty"]')).not.toBeNull();
      else expect(host.textContent).toContain("同步成功");
      return;
    case "/#/system":
    case "/#/system/status":
      expect(host.querySelector('[data-testid="system-status-layout"]')).not.toBeNull();
      return;
    case "/#/system/performance":
      expect(host.querySelector('[aria-label="Grafana"]')).not.toBeNull();
      return;
    case "/#/system/tasks":
      expect(host.querySelector("[data-task-catalog-row]")).not.toBeNull();
      return;
    case "/#/system/tasks/retention_archive":
      expect(host.querySelector('[data-testid="task-workload-panels"]')).not.toBeNull();
      return;
    case "/#/system/prompt-cache":
      expect(
        host.querySelector('[data-testid="system-prompt-cache"] [role="switch"]'),
      ).not.toBeNull();
      return;
    case "/#/system/models":
      expect(host.textContent).toMatch(/\d+ 个模型/);
      return;
    case "/#/system/settings":
    case "/#/settings":
    case "/#/settings/legacy":
      expect(host.querySelector(".settings-page")?.textContent).toContain("代理配置");
      return;
    case "/#/system/proxy":
      expect(host.querySelector(".settings-page")?.textContent).toContain("正向代理路由");
      return;
    default:
      expectRouteLandmark(route);
  }
}

describe("Web Demo route matrix", () => {
  beforeAll(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      writable: true,
      value: (query: string) => ({
        matches: false,
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        addListener: vi.fn(),
        removeListener: vi.fn(),
        dispatchEvent: vi.fn(),
        onchange: null,
      }),
    });
    if (!previousEventSource) {
      const EventSourceCapabilityStub = Object.assign(function EventSourceCapabilityStub() {}, {
        CONNECTING: 0,
        OPEN: 1,
        CLOSED: 2,
      });
      Object.defineProperty(window, "EventSource", {
        configurable: true,
        writable: true,
        value: EventSourceCapabilityStub as unknown as typeof EventSource,
      });
    }
    if (typeof previousCss?.supports !== "function") {
      Object.defineProperty(globalThis, "CSS", {
        configurable: true,
        writable: true,
        value: { ...(previousCss ?? {}), supports: () => true },
      });
    }
    if (typeof previousGetBBox !== "function") {
      Object.defineProperty(SVGElement.prototype, "getBBox", {
        configurable: true,
        writable: true,
        value() {
          return {
            x: 0,
            y: 0,
            width: (this.textContent?.length ?? 0) * 8,
            height: 16,
            top: 0,
            right: (this.textContent?.length ?? 0) * 8,
            bottom: 16,
            left: 0,
          };
        },
      });
    }
    server.listen({
      onUnhandledRequest: ({ method, url }) => {
        unhandledRequests.push(`${method} ${url}`);
      },
    });
    cleanupFetch = installDemoFetchFallback(handleDemoRequest);
    cleanupEventSource = installDemoEventSource();
    const createDemoEventSource = window.__CVM_DEMO_CREATE_EVENT_SOURCE__;
    if (createDemoEventSource) {
      window.__CVM_DEMO_CREATE_EVENT_SOURCE__ = (path) => {
        const source = createDemoEventSource(path);
        source.addEventListener("error", () => {
          failedDemoRequests.push(`GET ${new URL(path, window.location.href).pathname}`);
        });
        return source;
      };
    }
    const fallbackFetch = window.fetch;
    window.fetch = async (input, init) => {
      const request =
        input instanceof Request && init == null
          ? input
          : new Request(new URL(input.toString(), window.location.href), init);
      const response = await fallbackFetch(input, init);
      const pathname = new URL(request.url).pathname;
      if (demoModel.snapshot.scene === "network-failure" && pathname.startsWith("/api/")) {
        failedDemoRequests.push(`${request.method} ${pathname}`);
        throw new TypeError("Failed to fetch");
      }
      if (pathname.startsWith("/api/") && response.status === 501) {
        unexpectedDemoApiResponses.push(`${request.method} ${pathname}`);
      }
      return response;
    };
    cleanupFetchAudit = () => {
      window.fetch = fallbackFetch;
      unexpectedDemoApiResponses.length = 0;
      failedDemoRequests.length = 0;
    };
  });

  afterAll(() => {
    act(() => root.unmount());
    host.remove();
    cleanupEventSource();
    cleanupFetchAudit();
    cleanupFetch();
    server.close();
    if (previousMatchMedia) {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        writable: true,
        value: previousMatchMedia,
      });
    } else {
      Reflect.deleteProperty(window, "matchMedia");
    }
    if (previousEventSource) {
      Object.defineProperty(window, "EventSource", {
        configurable: true,
        writable: true,
        value: previousEventSource,
      });
    } else {
      Reflect.deleteProperty(window, "EventSource");
    }
    if (previousCss) {
      Object.defineProperty(globalThis, "CSS", {
        configurable: true,
        writable: true,
        value: previousCss,
      });
    } else {
      Reflect.deleteProperty(globalThis, "CSS");
    }
    if (previousGetBBox) {
      Object.defineProperty(SVGElement.prototype, "getBBox", {
        configurable: true,
        writable: true,
        value: previousGetBBox,
      });
    } else {
      Reflect.deleteProperty(SVGElement.prototype, "getBBox");
    }
  });

  it("keeps the 26 routes x 4 scenes semantic matrix on the real application", async () => {
    expect(demoRoutes).toHaveLength(26);
    expect(new Set(demoRoutes.map((route) => route.path)).size).toBe(26);
    expect(demoSemanticMatrix).toHaveLength(104);

    for (const { route, scene } of demoSemanticMatrix) {
      unexpectedDemoApiResponses.length = 0;
      unhandledRequests.length = 0;
      failedDemoRequests.length = 0;
      renderRoute(route.path, scene);
      const assertCurrentRoute = () => {
        assertRouteTerminalState(route, scene);
        expect(host.querySelector("[data-testid=demo-route-probe]")?.textContent).toBe(
          route.expectedPath,
        );
        expect(host.querySelector('[role="alert"] h1')?.textContent).not.toBe("页面加载失败");
        expect(unexpectedDemoApiResponses, `${route.path} / ${scene}`).toEqual([]);
        expect(unhandledRequests, `${route.path} / ${scene}`).toEqual([]);
      };
      await vi.waitFor(assertCurrentRoute, { interval: 1, timeout: 15_000 });
      act(() => root.unmount());
      root = createRoot(host);
    }
  }, 120_000);
});
