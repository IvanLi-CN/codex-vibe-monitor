/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  UpstreamAccountDetail,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
} from "../lib/api";
import { useUpstreamAccounts } from "./useUpstreamAccounts";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccounts: vi.fn<() => Promise<UpstreamAccountListResponse>>(),
  fetchUpstreamAccountDetail: vi.fn<
    (accountId: number, signal?: AbortSignal) => Promise<UpstreamAccountDetail>
  >(),
  syncUpstreamAccount: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccounts: apiMocks.fetchUpstreamAccounts,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    syncUpstreamAccount: apiMocks.syncUpstreamAccount,
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
  apiMocks.fetchUpstreamAccounts.mockResolvedValue(createListResponse());
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.clearAllMocks();
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
    routing: { apiKeyConfigured: false, maskedApiKey: null },
  };
}

function Probe() {
  const { selectedId, selectedSummary, detail, isDetailLoading, error, selectAccount, runSync } =
    useUpstreamAccounts();

  return (
    <div>
      <div data-testid="selected-id">{selectedId ?? ""}</div>
      <div data-testid="selected-name">{selectedSummary?.displayName ?? ""}</div>
      <div data-testid="detail-id">{detail?.id ?? ""}</div>
      <div data-testid="detail-name">{detail?.displayName ?? ""}</div>
      <div data-testid="detail-loading">{isDetailLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <button data-testid="select-beta" onClick={() => selectAccount(2)}>
        select beta
      </button>
      <button data-testid="sync-alpha" onClick={() => void runSync(1)}>
        sync alpha
      </button>
    </div>
  );
}

describe("useUpstreamAccounts", () => {
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
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
  });
});
