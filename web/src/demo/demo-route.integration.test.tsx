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

function assertRouteLandmark(route: (typeof demoRoutes)[number], scene: DemoScene) {
  if (route.path === "/#/account-pool/groups") {
    expect(
      host.querySelector('[data-testid="account-pool-groups-list"]') ??
        host.querySelector('[data-testid="account-pool-groups-empty"]') ??
        (scene === "network-failure"
          ? host.querySelector('[data-testid="account-pool-groups-error"]')
          : null),
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
  if (scene === "network-failure") {
    const landmark =
      route.landmark.kind === "testid"
        ? (host.querySelector(`[data-testid="${route.landmark.value}"]`) ??
          document.body.querySelector(`[data-testid="${route.landmark.value}"]`) ??
          host.querySelector(
            `[data-testid="${route.landmark.value.replace(/-list$/, "-error")}"]`,
          ) ??
          document.body.querySelector(
            `[data-testid="${route.landmark.value.replace(/-list$/, "-error")}"]`,
          ))
        : Array.from(document.body.querySelectorAll("h1, h2, h3, h4, h5, h6")).find(
            (heading) => heading.textContent?.trim() === route.landmark.value,
          );
    expect(
      landmark ??
        document.body.querySelector('[role="alert"]') ??
        Array.from(document.body.querySelectorAll('[role="status"]')).find((element) =>
          element.textContent?.includes("加载系统状态失败"),
        ) ??
        host.querySelector('[data-testid="dashboard-main-content"]') ??
        (route.path === "/#/system"
          ? Array.from(host.querySelectorAll("h1, h2, h3, h4, h5, h6")).find(
              (heading) => heading.textContent?.trim() === "系统状态",
            )
          : null),
      `${route.path} / ${scene}`,
    ).not.toBeNull();
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
    const fallbackFetch = window.fetch;
    window.fetch = async (input, init) => {
      const request =
        input instanceof Request && init == null
          ? input
          : new Request(new URL(input.toString(), window.location.href), init);
      const response = await fallbackFetch(input, init);
      const pathname = new URL(request.url).pathname;
      if (demoModel.snapshot.scene === "network-failure" && pathname.startsWith("/api/")) {
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
      renderRoute(route.path, scene);
      const assertCurrentRoute = () => {
        assertRouteLandmark(route, scene);
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
