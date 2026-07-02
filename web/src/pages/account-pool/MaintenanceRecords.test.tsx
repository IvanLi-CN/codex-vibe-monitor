/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import MaintenanceRecordsPage from "./MaintenanceRecords";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountActionEvents: vi.fn(),
}));
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountActionEvents: apiMocks.fetchUpstreamAccountActionEvents,
  };
});

vi.mock("../../hooks/useUpstreamAccounts", () => ({
  useUpstreamAccounts: hookMocks.useUpstreamAccounts,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;
const inputValueSetter = Object.getOwnPropertyDescriptor(
  HTMLInputElement.prototype,
  "value",
)?.set;

function buildResponse(accountDisplayName: string) {
  return {
    total: 1,
    page: 1,
    pageSize: 20,
    items: [
      {
        id: 1,
        accountId: 11,
        accountDisplayName,
        accountGroupName: "production",
        forwardProxyKey: "__direct__",
        forwardProxyDisplayName: "Direct",
        forwardProxyEgressIp: null,
        action: "sync_succeeded",
        result: "success",
        reasonCode: "sync_ok",
        reasonMessage: null,
        resultDescription: null,
        httpStatus: null,
        invocationId: null,
        occurredAt: "2026-07-02T01:02:03Z",
      },
    ],
  };
}

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider>
        <MaintenanceRecordsPage />
      </I18nProvider>,
    );
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("MaintenanceRecordsPage", () => {
  beforeEach(() => {
    hookMocks.useUpstreamAccounts.mockReturnValue({
      forwardProxyNodes: [],
    });
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    apiMocks.fetchUpstreamAccountActionEvents.mockReset();
    hookMocks.useUpstreamAccounts.mockReset();
  });

  it("keeps stale rows visible and marks the table body while refetching", async () => {
    let resolveInitial:
      | ((value: ReturnType<typeof buildResponse>) => void)
      | null = null;
    const initialRequest = new Promise<ReturnType<typeof buildResponse>>((resolve) => {
      resolveInitial = resolve;
    });
    const refetchRequest = new Promise<ReturnType<typeof buildResponse>>(() => undefined);
    apiMocks.fetchUpstreamAccountActionEvents
      .mockReturnValueOnce(initialRequest)
      .mockReturnValueOnce(refetchRequest);

    renderPage();
    expect(host?.querySelector('[data-testid="maintenance-records-loading"]')).not.toBeNull();

    await act(async () => {
      resolveInitial?.(buildResponse("Existing OAuth"));
      await initialRequest;
    });
    await flushAsync();

    expect(host?.textContent ?? "").toContain("Existing OAuth");

    const accountInput = host?.querySelector(
      'input[placeholder="搜索账号名或 ID"]',
    ) as HTMLInputElement | null;
    expect(accountInput).not.toBeNull();

    await act(async () => {
      inputValueSetter?.call(accountInput, "new filter");
      accountInput!.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActionEvents).toHaveBeenCalledTimes(2);
    expect(host?.querySelector('[data-testid="maintenance-records-refreshing"]')).not.toBeNull();
    expect(host?.textContent ?? "").toContain("Existing OAuth");
  });

  it("keeps stale rows visible and shows an inline warning when refetching fails", async () => {
    apiMocks.fetchUpstreamAccountActionEvents
      .mockResolvedValueOnce(buildResponse("Existing OAuth"))
      .mockRejectedValueOnce(new Error("Network failed"));

    renderPage();
    await flushAsync();

    expect(host?.textContent ?? "").toContain("Existing OAuth");

    const accountInput = host?.querySelector(
      'input[placeholder="搜索账号名或 ID"]',
    ) as HTMLInputElement | null;
    expect(accountInput).not.toBeNull();

    await act(async () => {
      inputValueSetter?.call(accountInput, "new filter");
      accountInput!.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActionEvents).toHaveBeenCalledTimes(2);
    expect(host?.textContent ?? "").toContain("Network failed");
    expect(host?.textContent ?? "").toContain("Existing OAuth");
    expect(host?.querySelector('[data-testid="maintenance-records-error"]')).toBeNull();
  });
});
