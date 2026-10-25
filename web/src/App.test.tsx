/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Outlet, useLocation } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import App, { AppErrorBoundary } from "./App";

vi.mock("./features/app-shell/AppLayout", () => ({
  AppLayout: () => <Outlet />,
}));

vi.mock("./pages/Dashboard", () => ({
  default: () => <div>dashboard page</div>,
}));

vi.mock("./pages/Live", () => ({
  default: () => <div>live page</div>,
}));

vi.mock("./pages/Records", () => ({
  default: () => <div>records page</div>,
}));

vi.mock("./pages/Settings", () => ({
  default: ({ mode }: { mode?: string }) => <div>{`legacy settings page ${mode ?? "all"}`}</div>,
}));

vi.mock("./pages/Stats", () => ({
  default: () => <div>stats page</div>,
}));

vi.mock("./pages/account-pool/AccountPoolLayout", () => ({
  default: () => <Outlet />,
}));

vi.mock("./pages/account-pool/Groups", () => ({
  default: () => <div>groups page</div>,
}));

vi.mock("./pages/account-pool/MaintenanceRecords", () => ({
  default: () => <div>maintenance records page</div>,
}));

vi.mock("./pages/account-pool/UpstreamAccounts", () => ({
  default: () => <div>upstream accounts page</div>,
}));

vi.mock("./pages/account-pool/UpstreamAccountCreate", () => ({
  default: () => <div>upstream account create page</div>,
}));

vi.mock("./pages/account-pool/Tags", () => ({
  default: () => <div>tags page</div>,
}));

vi.mock("./pages/system/SystemLayout", () => ({
  default: () => <Outlet />,
}));

vi.mock("./pages/system/SystemStatusPage", () => ({
  default: () => <div>system status page</div>,
}));

vi.mock("./pages/system/SystemTasksPage", () => ({
  default: () => <div>system tasks page</div>,
}));

vi.mock("./pages/system/SystemSettingsPage", () => ({
  default: () => <div>system settings page</div>,
}));
vi.mock("./pages/system/SystemPerformancePage", () => ({
  default: () => <div>system performance page</div>,
}));
vi.mock("./pages/system/SystemModelsPage", () => ({
  default: () => <div>system models page</div>,
}));

vi.mock("./pages/system/SystemPromptCachePage", () => ({
  default: () => <div>system prompt cache page</div>,
}));

vi.mock("./pages/system/SystemProxyPage", () => ({
  default: () => <div>system proxy page</div>,
}));

vi.mock("./pages/system/SystemTaskDetailPage", () => ({
  default: () => <div>system task detail page</div>,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function LocationProbe() {
  const location = useLocation();
  return (
    <>
      <div data-testid="location">{location.pathname}</div>
      <div data-testid="location-search">{location.search}</div>
    </>
  );
}

function renderApp(entry: string) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);

  act(() => {
    root?.render(
      <MemoryRouter initialEntries={[entry]}>
        <App />
        <LocationProbe />
      </MemoryRouter>,
    );
  });
}

describe("App routes", () => {
  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
  });

  it.each([
    ["root", "/", "/dashboard", "", "dashboard page"],
    ["dashboard", "/dashboard", "/dashboard", "", "dashboard page"],
    [
      "dashboard invocation detail",
      "/dashboard/invocations/invoke-9002",
      "/dashboard/invocations/invoke-9002",
      "",
      "dashboard page",
    ],
    [
      "dashboard query",
      "/dashboard?tab=attention",
      "/dashboard",
      "?tab=attention",
      "dashboard page",
    ],
    ["stats", "/stats", "/stats", "", "stats page"],
    ["live", "/live", "/live", "", "live page"],
    ["records", "/records", "/records", "", "records page"],
    [
      "account-pool default",
      "/account-pool",
      "/account-pool/transits",
      "",
      "upstream accounts page",
    ],
    [
      "account-pool transits",
      "/account-pool/transits",
      "/account-pool/transits",
      "",
      "upstream accounts page",
    ],
    [
      "account-pool transit create",
      "/account-pool/transits/new",
      "/account-pool/transits/new",
      "",
      "upstream account create page",
    ],
    ["account-pool pool", "/account-pool/pool", "/account-pool/pool", "", "upstream accounts page"],
    [
      "account-pool pool create",
      "/account-pool/pool/new",
      "/account-pool/pool/new",
      "",
      "upstream account create page",
    ],
    [
      "legacy account list",
      "/account-pool/upstream-accounts",
      "/account-pool/pool",
      "",
      "upstream accounts page",
    ],
    [
      "legacy transit account create",
      "/account-pool/upstream-accounts/new?mode=apiKey",
      "/account-pool/transits/new",
      "?mode=apiKey",
      "upstream account create page",
    ],
    [
      "legacy pool account create",
      "/account-pool/upstream-accounts/new?mode=oauth",
      "/account-pool/pool/new",
      "?mode=oauth",
      "upstream account create page",
    ],
    [
      "legacy account create default",
      "/account-pool/upstream-accounts/new",
      "/account-pool/pool/new",
      "",
      "upstream account create page",
    ],
    [
      "maintenance records",
      "/account-pool/maintenance-records",
      "/account-pool/maintenance-records",
      "",
      "maintenance records page",
    ],
    ["account groups", "/account-pool/groups", "/account-pool/groups", "", "groups page"],
    ["system default", "/system", "/system/status", "", "system status page"],
    ["system status", "/system/status", "/system/status", "", "system status page"],
    [
      "system performance",
      "/system/performance",
      "/system/performance",
      "",
      "system performance page",
    ],
    ["system tasks", "/system/tasks", "/system/tasks", "", "system tasks page"],
    [
      "system task detail",
      "/system/tasks/retention_archive",
      "/system/tasks/retention_archive",
      "",
      "system task detail page",
    ],
    [
      "system prompt cache",
      "/system/prompt-cache",
      "/system/prompt-cache",
      "",
      "system prompt cache page",
    ],
    ["system settings", "/system/settings", "/system/settings", "", "system settings page"],
    ["system models", "/system/models", "/system/models", "", "system models page"],
    ["system proxy", "/system/proxy", "/system/proxy", "", "system proxy page"],
    ["legacy settings", "/settings", "/system/settings", "", "system settings page"],
    [
      "legacy settings path",
      "/settings/legacy?tab=general",
      "/system/settings",
      "",
      "system settings page",
    ],
    ["unknown route", "/not-a-route", "/dashboard", "", "dashboard page"],
  ] as const)("$0 resolves $1", async (_name, entry, expectedPath, expectedSearch, expectedText) => {
    await act(async () => {
      renderApp(entry);
    });

    expect(host?.textContent ?? "").toContain(expectedText);
    expect(host?.querySelector('[data-testid="location"]')?.textContent).toBe(expectedPath);
    expect(host?.querySelector('[data-testid="location-search"]')?.textContent).toBe(
      expectedSearch,
    );
  });

  it("keeps a rejected lazy route recoverable", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    function RejectedRoute() {
      throw new Error("chunk failed");
    }

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);

    await act(async () => {
      root?.render(
        <AppErrorBoundary>
          <RejectedRoute />
        </AppErrorBoundary>,
      );
    });

    expect(host.textContent).toContain("页面加载失败");
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
    expect(host.textContent).toContain("重新加载");
    consoleError.mockRestore();
  });
});
