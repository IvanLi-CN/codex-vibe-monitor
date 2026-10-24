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
            <MemoryRouter initialEntries={[{ pathname, search }]}>
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
        host.querySelector('[data-testid="account-pool-groups-empty"]'),
      route.path,
    ).not.toBeNull();
    return;
  }
  if (route.path === "/#/system/tasks/retention_archive") {
    expect(
      host.querySelector("h1, h2, h3, h4, h5, h6")?.textContent?.includes("数据保留与归档") ??
        host.textContent?.includes("任务观测不可用"),
      route.path,
    ).toBe(true);
    return;
  }
  if (scene === "network-failure") {
    expect(
      host.querySelector(`[data-testid="${route.landmark.value}"]`) ??
        host.querySelector('[role="alert"]'),
      `${route.path} / ${scene}`,
    ).not.toBeNull();
    return;
  }
  if (route.landmark.kind === "testid") {
    expect(
      host.querySelector(`[data-testid="${route.landmark.value}"]`),
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
  });

  it("keeps the 26 routes x 4 scenes semantic matrix on the real application", async () => {
    expect(demoRoutes).toHaveLength(26);
    expect(new Set(demoRoutes.map((route) => route.path)).size).toBe(26);
    expect(demoSemanticMatrix).toHaveLength(104);

    for (const { route, scene } of demoSemanticMatrix) {
      unexpectedDemoApiResponses.length = 0;
      unhandledRequests.length = 0;
      renderRoute(route.path, scene);
      await vi.waitFor(
        () => {
          assertRouteLandmark(route, scene);
          expect(host.querySelector("[data-testid=demo-route-probe]")?.textContent).toBe(
            route.expectedPath,
          );
          expect(host.querySelector('[role="alert"] h1')?.textContent).not.toBe("页面加载失败");
          expect(unexpectedDemoApiResponses, `${route.path} / ${scene}`).toEqual([]);
          expect(unhandledRequests, `${route.path} / ${scene}`).toEqual([]);
        },
        { interval: 1, timeout: 5_000 },
      );
      act(() => root.unmount());
      root = createRoot(host);
    }
  }, 120_000);
});
