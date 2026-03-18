/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FetchUpstreamAccountsQuery,
  UpstreamAccountDetail,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
} from "../lib/api";
import { useUpstreamAccounts } from "./useUpstreamAccounts";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccounts: vi.fn<
    (query?: FetchUpstreamAccountsQuery) => Promise<UpstreamAccountListResponse>
  >(),
  fetchUpstreamAccountDetail: vi.fn<
    (accountId: number, signal?: AbortSignal) => Promise<UpstreamAccountDetail>
  >(),
  syncUpstreamAccount: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  reloginUpstreamAccount: vi.fn<(accountId: number) => Promise<{ loginId: string }>>(),
  deleteUpstreamAccount: vi.fn<(accountId: number) => Promise<void>>(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccounts: apiMocks.fetchUpstreamAccounts,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    syncUpstreamAccount: apiMocks.syncUpstreamAccount,
    reloginUpstreamAccount: apiMocks.reloginUpstreamAccount,
    deleteUpstreamAccount: apiMocks.deleteUpstreamAccount,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

beforeEach(() => {
  vi.resetAllMocks();
  apiMocks.fetchUpstreamAccounts.mockResolvedValue(createListResponse());
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function click(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLButtonElement)) {
    throw new Error(`Missing button: ${testId}`);
  }
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function createSummary(
  id: number,
  displayName: string,
): UpstreamAccountSummary {
  return {
    id,
    kind: "oauth_codex",
    provider: "codex",
    displayName,
    groupName: "prod",
    isMother: false,
    status: "active",
    enabled: true,
    tags: [],
    effectiveRoutingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    },
  };
}

function createDetail(id: number, displayName: string): UpstreamAccountDetail {
  return {
    ...createSummary(id, displayName),
    email: `${displayName.toLowerCase().replace(/\s+/g, ".")}@example.com`,
    history: [],
  };
}

function createListResponse(): UpstreamAccountListResponse {
  return {
    writesEnabled: true,
    items: [createSummary(1, "Alpha"), createSummary(2, "Beta")],
    groups: [],
    hasUngroupedAccounts: false,
    routing: { apiKeyConfigured: false, maskedApiKey: null },
  };
}

function Probe({ query }: { query?: FetchUpstreamAccountsQuery }) {
  const {
    selectedId,
    selectedSummary,
    detail,
    isDetailLoading,
    listError,
    detailError,
    error,
    selectAccount,
    runSync,
    refresh,
    beginRelogin,
    removeAccount,
  } =
    useUpstreamAccounts(query);

  return (
    <div>
      <div data-testid="selected-id">{selectedId ?? ""}</div>
      <div data-testid="selected-name">{selectedSummary?.displayName ?? ""}</div>
      <div data-testid="detail-id">{detail?.id ?? ""}</div>
      <div data-testid="detail-name">{detail?.displayName ?? ""}</div>
      <div data-testid="detail-loading">{isDetailLoading ? "true" : "false"}</div>
      <div data-testid="list-error">{listError ?? ""}</div>
      <div data-testid="detail-error">{detailError ?? ""}</div>
      <div data-testid="error">{error ?? ""}</div>
      <button data-testid="select-beta" onClick={() => selectAccount(2)}>
        select beta
      </button>
      <button data-testid="select-alpha" onClick={() => selectAccount(1)}>
        select alpha
      </button>
      <button data-testid="select-gamma" onClick={() => selectAccount(3)}>
        select gamma
      </button>
      <button data-testid="sync-alpha" onClick={() => void runSync(1)}>
        sync alpha
      </button>
      <button data-testid="refresh" onClick={() => void refresh()}>
        refresh
      </button>
      <button data-testid="relogin-alpha" onClick={() => void beginRelogin(1)}>
        relogin alpha
      </button>
      <button data-testid="remove-alpha" onClick={() => void removeAccount(1)}>
        remove alpha
      </button>
    </div>
  );
}

describe("useUpstreamAccounts", () => {
  it("passes server-side roster filters through to the list endpoint", async () => {
    render(<Probe query={{ groupSearch: "prod", tagIds: [1, 2] }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledWith({
      groupSearch: "prod",
      tagIds: [1, 2],
    });
  });

  it("ignores stale detail responses after account switches", async () => {
    const first = deferred<UpstreamAccountDetail>();
    const second = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    click("select-beta");
    await flushAsync();

    second.resolve(createDetail(2, "Beta"));
    await flushAsync();
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");

    first.resolve(createDetail(1, "Alpha"));
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
  });

  it("ignores stale detail errors after account switches", async () => {
    const first = deferred<UpstreamAccountDetail>();
    const second = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    await flushAsync();

    second.resolve(createDetail(2, "Beta"));
    await flushAsync();

    first.reject(new Error("Alpha failed"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
    expect(text("error")).toBe("");
  });

  it("invalidates the previous detail request in the same turn as a selection change", async () => {
    const first = deferred<UpstreamAccountDetail>();
    const second = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    first.reject(new Error("Alpha failed"));
    await flushAsync();

    second.resolve(createDetail(2, "Beta"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
    expect(text("error")).toBe("");
  });

  it("does not reclaim selection when an older account sync finishes later", async () => {
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta"))
      .mockResolvedValue(createDetail(2, "Beta"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");

    click("sync-alpha");
    click("select-beta");
    await flushAsync();
    expect(text("selected-id")).toBe("2");

    sync.resolve(createDetail(1, "Alpha"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("selected-name")).toBe("Beta");
    expect(text("detail-id")).not.toBe("1");
    expect(text("detail-name")).not.toBe("Alpha");
  });

  it("reloads the currently selected account detail after another account sync finishes", async () => {
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce(createListResponse());
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta Stale"))
      .mockResolvedValueOnce(createDetail(2, "Beta Fresh"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("sync-alpha");
    click("select-beta");
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta Stale");

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      3,
      2,
      expect.any(AbortSignal),
    );
    expect(text("selected-id")).toBe("2");
  });

  it("keeps synced detail when an older detail refresh resolves afterwards", async () => {
    const refreshedDetail = deferred<UpstreamAccountDetail>();
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async () => refreshedDetail.promise)
      .mockResolvedValue(createDetail(1, "Alpha Synced"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    click("sync-alpha");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Synced");

    refreshedDetail.resolve(createDetail(1, "Alpha Stale"));
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Synced");
  });

  it("does not clear the current account error when another account sync succeeds", async () => {
    const betaFailure = deferred<UpstreamAccountDetail>();
    const betaRefresh = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async () => betaFailure.promise)
      .mockImplementationOnce(async () => betaRefresh.promise);
    apiMocks.syncUpstreamAccount.mockResolvedValueOnce(createDetail(1, "Alpha Synced"));

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    await flushAsync();
    betaFailure.reject(new Error("Beta failed"));
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("error")).toBe("Beta failed");

    click("sync-alpha");
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("error")).toBe("Beta failed");
  });

  it("refreshes detail using the list's final selection", async () => {
    const betaDetail = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce({
        writesEnabled: true,
        items: [createSummary(2, "Beta")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { apiKeyConfigured: false, maskedApiKey: null },
      });
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async (accountId: number) => {
        if (accountId !== 2) {
          throw new Error(`unexpected account ${accountId}`);
        }
        return betaDetail.promise;
      })
      .mockResolvedValue(createDetail(2, "Beta"));

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      2,
      2,
      expect.any(AbortSignal),
    );

    betaDetail.resolve(createDetail(2, "Beta"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
    expect(text("error")).toBe("");
  });

  it("keeps the current detail when list refresh fails", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");
    expect(text("detail-name")).toBe("Alpha");

    click("refresh");
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");
    expect(text("detail-name")).toBe("Alpha");
    expect(text("error")).toBe("List failed");
  });

  it("keeps list and detail errors visible independently", async () => {
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockRejectedValueOnce(new Error("Beta failed"));
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    await flushAsync();

    expect(text("detail-error")).toBe("Beta failed");
    expect(text("list-error")).toBe("");

    click("refresh");
    await flushAsync();

    expect(text("detail-error")).toBe("Beta failed");
    expect(text("list-error")).toBe("List failed");
  });

  it("keeps account detail errors scoped per account", async () => {
    apiMocks.fetchUpstreamAccountDetail
      .mockRejectedValueOnce(new Error("Alpha failed"))
      .mockRejectedValueOnce(new Error("Beta failed"))
      .mockRejectedValueOnce(new Error("Alpha failed"));

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("Alpha failed");

    click("select-beta");
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("detail-error")).toBe("Beta failed");

    click("select-alpha");
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("Alpha failed");
  });

  it("does not clear list errors after a non-list success", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));
    apiMocks.reloginUpstreamAccount.mockResolvedValueOnce({ loginId: "relogin-1" });

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    expect(text("list-error")).toBe("List failed");

    click("relogin-alpha");
    await flushAsync();
    expect(text("list-error")).toBe("List failed");
  });

  it("clears an account error after that account sync succeeds off-selection", async () => {
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockRejectedValueOnce(new Error("Alpha failed"))
      .mockResolvedValueOnce(createDetail(2, "Beta"))
      .mockResolvedValueOnce(createDetail(2, "Beta"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("Alpha failed");

    click("sync-alpha");
    click("select-beta");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha synced"));
    await flushAsync();

    click("select-alpha");
    await flushAsync();
    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("");
  });

  it("does not reclaim selection when a delete finishes after switching away", async () => {
    const remove = deferred<void>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta"));
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce({
        writesEnabled: true,
        items: [createSummary(1, "Alpha"), createSummary(3, "Gamma"), createSummary(2, "Beta")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { apiKeyConfigured: false, maskedApiKey: null },
      })
      .mockResolvedValueOnce({
        writesEnabled: true,
        items: [createSummary(2, "Beta"), createSummary(3, "Gamma")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { apiKeyConfigured: false, maskedApiKey: null },
      });
    apiMocks.deleteUpstreamAccount.mockImplementationOnce(async () => remove.promise);

    render(<Probe />);
    await flushAsync();

    click("remove-alpha");
    click("select-beta");
    await flushAsync();

    remove.resolve();
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("selected-name")).toBe("Beta");
  });

  it("reanchors away from a deleted current account even if the list refresh fails", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta"));
    apiMocks.deleteUpstreamAccount.mockResolvedValueOnce();

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");

    click("remove-alpha");
    await flushAsync();
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("selected-name")).toBe("Beta");
    expect(text("detail-id")).not.toBe("1");
    expect(text("detail-name")).not.toBe("Alpha");
  });

  it("invalidates an older detail reload before sync refreshes the list", async () => {
    const refreshedDetail = deferred<UpstreamAccountDetail>();
    const syncedList = deferred<UpstreamAccountListResponse>();
    const sync = deferred<UpstreamAccountDetail>();

    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => syncedList.promise);
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async () => refreshedDetail.promise)
      .mockResolvedValue(createDetail(1, "Alpha Synced"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    click("sync-alpha");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();

    refreshedDetail.resolve(createDetail(1, "Alpha Stale"));
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha");

    syncedList.resolve(createListResponse());
    await flushAsync();
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Synced");
  });

  it("refreshes the final selected account after switching during refresh", async () => {
    const refreshedList = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => refreshedList.promise);
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta Stale"))
      .mockResolvedValueOnce(createDetail(2, "Beta Fresh"));

    render(<Probe />);
    await flushAsync();

    click("refresh");
    click("select-beta");
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta Stale");

    refreshedList.resolve(createListResponse());
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      3,
      2,
      expect.any(AbortSignal),
    );
    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta Fresh");
  });

  it("ignores an older list refresh after sync starts a newer list reload", async () => {
    const staleRefreshList = deferred<UpstreamAccountListResponse>();
    const syncedList = deferred<UpstreamAccountListResponse>();
    const sync = deferred<UpstreamAccountDetail>();

    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => staleRefreshList.promise)
      .mockImplementationOnce(async () => syncedList.promise)
      .mockResolvedValue({
        writesEnabled: true,
        items: [createSummary(1, "Alpha Synced"), createSummary(2, "Beta")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { apiKeyConfigured: false, maskedApiKey: null },
      });
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValue(createDetail(1, "Alpha Synced"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    click("sync-alpha");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();

    syncedList.resolve({
      writesEnabled: true,
      items: [createSummary(1, "Alpha Synced"), createSummary(2, "Beta")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: { apiKeyConfigured: false, maskedApiKey: null },
    });
    await flushAsync();
    await flushAsync();

    expect(text("selected-name")).toBe("Alpha Synced");
    expect(text("detail-name")).toBe("Alpha Synced");

    staleRefreshList.resolve({
      writesEnabled: true,
      items: [createSummary(1, "Alpha Stale"), createSummary(2, "Beta")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: { apiKeyConfigured: false, maskedApiKey: null },
    });
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("selected-name")).toBe("Alpha Synced");
    expect(text("detail-name")).toBe("Alpha Synced");
  });
});
