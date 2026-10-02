/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemPerformancePage from "./SystemPerformancePage";

const mocks = vi.hoisted(() => ({ capabilities: vi.fn() }));
vi.mock("../../lib/api", async () => ({
  ...(await vi.importActual<typeof import("../../lib/api")>("../../lib/api")),
  fetchObservabilityCapabilities: mocks.capabilities,
}));
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  mocks.capabilities.mockResolvedValue({
    enabled: true,
    state: "enabled",
    grafanaPublicUrl: "https://grafana.example.invalid/monitor",
    grafanaConnectivity: "unknown",
  });
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.clearAllMocks();
});
async function render(): Promise<void> {
  await act(async () =>
    root.render(
      <I18nProvider>
        <SystemPerformancePage />
      </I18nProvider>,
    ),
  );
}
describe("external observability entry", () => {
  it("opens only the five provisioned dashboards and keeps connectivity unknown", async () => {
    await render();
    const links = Array.from(host.querySelectorAll<HTMLAnchorElement>("nav a"));
    expect(links).toHaveLength(5);
    expect(links[0].href).toContain("/monitor/d/cvm-overview?");
    expect(links[0].search).toContain("timezone=utc");
    expect(host.querySelector("svg")).toBeNull();
    expect(host.textContent).toMatch(/unknown|未知/);
  });
  it("shows configuration absence without a fallback chart", async () => {
    mocks.capabilities.mockResolvedValue({
      enabled: true,
      state: "enabled",
      grafanaPublicUrl: null,
    });
    await render();
    expect(host.querySelector("nav")).toBeNull();
    expect(host.textContent).toMatch(/configured|配置/);
    expect(host.querySelector('[data-testid="performance-series-chart"]')).toBeNull();
  });
  it("shows local status failure without claiming connectivity", async () => {
    mocks.capabilities.mockRejectedValue(new Error("unavailable"));
    await render();
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
    expect(host.querySelector("a")).toBeNull();
  });
});
