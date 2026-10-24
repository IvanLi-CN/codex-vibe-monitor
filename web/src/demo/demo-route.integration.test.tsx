/** @vitest-environment jsdom */

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
import { handleDemoRequest } from "./handlers";
import { demoModel } from "./model";
import { demoSemanticMatrix } from "./route-matrix";
import type { DemoScene } from "./runtime";

let host: HTMLDivElement;
let root: Root;
let cleanupFetch: () => void;
let cleanupEventSource: () => void;

function RouteProbe() {
  const location = useLocation();
  return <output data-testid="demo-route-probe">{location.pathname}</output>;
}

function routeQuery(path: string): string {
  const hash = path.slice(path.indexOf("#") + 1);
  const queryIndex = hash.indexOf("?");
  return queryIndex === -1 ? "" : hash.slice(queryIndex);
}

function renderRoute(pathname: string, search: string, scene: DemoScene) {
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

describe("Web Demo route matrix", () => {
  beforeAll(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    cleanupFetch = installDemoFetchFallback(handleDemoRequest);
    cleanupEventSource = installDemoEventSource();
  });

  afterAll(() => {
    act(() => root.unmount());
    host.remove();
    cleanupEventSource();
    cleanupFetch();
  });

  it("keeps the 26 routes x 4 scenes semantic matrix on the real application", async () => {
    expect(demoRoutes).toHaveLength(26);
    expect(new Set(demoRoutes.map((route) => route.path)).size).toBe(26);
    expect(demoSemanticMatrix).toHaveLength(104);

    for (const { route, scene } of demoSemanticMatrix) {
      const query = routeQuery(route.path);
      renderRoute(route.expectedPath, query, scene);
      await vi.waitFor(
        () => {
          expect(host.textContent?.trim(), `${route.path} / ${scene}`).toBeTruthy();
          expect(host.querySelector("[data-testid=demo-route-probe]")?.textContent).toBe(
            route.expectedPath,
          );
          expect(host.querySelector('[role="alert"] h1')?.textContent).not.toBe("页面加载失败");
        },
        { interval: 1, timeout: 5_000 },
      );
      act(() => root.unmount());
      root = createRoot(host);
    }
  }, 120_000);
});
