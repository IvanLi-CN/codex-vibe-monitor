/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import ModelRoutingPage from "./ModelRouting";

const hookMocks = vi.hoisted(() => ({
  useModelRoutingLive: vi.fn(),
}));

vi.mock("../hooks/useCompactViewport", () => ({
  useCompactViewport: () => false,
}));

vi.mock("../hooks/useModelRoutingLive", () => ({
  useModelRoutingLive: hookMocks.useModelRoutingLive,
}));

vi.mock("../hooks/useUpstreamAccountDetailRoute", () => ({
  useUpstreamAccountDetailRoute: () => ({
    upstreamAccountId: null,
    upstreamAccountTab: null,
    upstreamAccountModel: null,
    openUpstreamAccount: vi.fn(),
    closeUpstreamAccount: vi.fn(),
  }),
}));

vi.mock("../features/live/ModelRoutingLivePanel", () => ({
  ModelRoutingLivePanel: () => <div data-testid="model-routing-live-panel">模型路由</div>,
}));

vi.mock("./account-pool/UpstreamAccounts", () => ({
  SharedUpstreamAccountDetailDrawer: () => <div data-testid="upstream-account-drawer" />,
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
  vi.clearAllMocks();
});

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <MemoryRouter>
        <ModelRoutingPage />
      </MemoryRouter>,
    );
  });
}

describe("ModelRoutingPage", () => {
  it("is a standalone model-routing surface without conversation content", () => {
    hookMocks.useModelRoutingLive.mockReturnValue({
      data: null,
      isLoading: false,
      error: null,
      refresh: vi.fn(),
    });

    renderPage();

    expect(host?.querySelector('[data-testid="model-routing-live-panel"]')).toBeTruthy();
    expect(host?.textContent).toContain("模型路由");
    expect(host?.textContent).not.toContain("对话");
    expect(hookMocks.useModelRoutingLive).toHaveBeenLastCalledWith(
      {
        window: "24h",
        model: undefined,
        state: undefined,
        limit: 100,
      },
      true,
    );
  });
});
