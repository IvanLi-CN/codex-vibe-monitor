/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ForwardProxySettings,
  PricingSettings,
  SettingsPayload,
} from "../lib/api";
import { UPSTREAM_ACCOUNTS_CHANGED_EVENT } from "../lib/upstreamAccountsEvents";
import { useSettings } from "./useSettings";

const apiMocks = vi.hoisted(() => ({
  fetchSettings: vi.fn<() => Promise<SettingsPayload>>(),
  updateForwardProxySettings: vi.fn<
    (payload: {
      proxyUrls: string[];
      subscriptionUrls: string[];
      subscriptionUpdateIntervalSecs: number;
    }) => Promise<ForwardProxySettings>
  >(),
  updatePricingSettings: vi.fn<(payload: PricingSettings) => Promise<PricingSettings>>(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchSettings: apiMocks.fetchSettings,
    updateForwardProxySettings: apiMocks.updateForwardProxySettings,
    updatePricingSettings: apiMocks.updatePricingSettings,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function createForwardProxySettings(
  overrides: Partial<ForwardProxySettings> = {},
): ForwardProxySettings {
  return {
    proxyUrls: ["http://initial-proxy.example.com"],
    subscriptionUrls: ["https://subs.example.com/a"],
    subscriptionUpdateIntervalSecs: 600,
    nodes: [],
    ...overrides,
  };
}

function createSettingsPayload(
  overrides: Partial<SettingsPayload> = {},
): SettingsPayload {
  return {
    forwardProxy: createForwardProxySettings(),
    pricing: {
      catalogVersion: "2026-04-12",
      entries: [],
    },
    ...overrides,
  };
}

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

function click(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLButtonElement)) {
    throw new Error(`Missing button: ${testId}`);
  }
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function Probe() {
  const { settings, error, isLoading, saveForwardProxy } = useSettings();

  return (
    <div>
      <div data-testid="loading">{String(isLoading)}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="proxy-urls">
        {settings?.forwardProxy.proxyUrls.join(",") ?? ""}
      </div>
      <button
        data-testid="save-forward-proxy"
        disabled={!settings}
        onClick={() => {
          if (!settings) return;
          void saveForwardProxy({
            ...settings.forwardProxy,
            proxyUrls: ["http://refreshed-proxy.example.com"],
            subscriptionUrls: [
              ...settings.forwardProxy.subscriptionUrls,
              "https://subs.example.com/b",
            ],
          });
        }}
      >
        save
      </button>
    </div>
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  apiMocks.fetchSettings.mockResolvedValue(createSettingsPayload());
  apiMocks.updatePricingSettings.mockResolvedValue({
    catalogVersion: "2026-04-12",
    entries: [],
  });
  apiMocks.updateForwardProxySettings.mockImplementation(async (payload) =>
    createForwardProxySettings({
      proxyUrls: payload.proxyUrls,
      subscriptionUrls: payload.subscriptionUrls,
      subscriptionUpdateIntervalSecs: payload.subscriptionUpdateIntervalSecs,
    }),
  );
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

describe("useSettings", () => {
  it("emits the upstream-accounts invalidation event after forward-proxy settings save succeeds", async () => {
    let eventCount = 0;
    const handleChanged = () => {
      eventCount += 1;
    };
    window.addEventListener(UPSTREAM_ACCOUNTS_CHANGED_EVENT, handleChanged);

    try {
      render(<Probe />);
      await flushAsync();

      expect(text("loading")).toBe("false");
      expect(text("proxy-urls")).toContain("http://initial-proxy.example.com");

      click("save-forward-proxy");
      await flushAsync();

      expect(apiMocks.updateForwardProxySettings).toHaveBeenCalledTimes(1);
      expect(eventCount).toBe(1);
      expect(text("proxy-urls")).toContain("http://refreshed-proxy.example.com");
      expect(text("error")).toBe("");
    } finally {
      window.removeEventListener(
        UPSTREAM_ACCOUNTS_CHANGED_EVENT,
        handleChanged,
      );
    }
  });

  it("does not emit the invalidation event when saving forward-proxy settings fails", async () => {
    apiMocks.updateForwardProxySettings.mockRejectedValueOnce(
      new Error("save failed"),
    );

    let eventCount = 0;
    const handleChanged = () => {
      eventCount += 1;
    };
    window.addEventListener(UPSTREAM_ACCOUNTS_CHANGED_EVENT, handleChanged);

    try {
      render(<Probe />);
      await flushAsync();

      click("save-forward-proxy");
      await flushAsync();

      expect(apiMocks.updateForwardProxySettings).toHaveBeenCalledTimes(1);
      expect(eventCount).toBe(0);
      expect(text("error")).toBe("save failed");
      expect(text("proxy-urls")).toContain("http://initial-proxy.example.com");
    } finally {
      window.removeEventListener(
        UPSTREAM_ACCOUNTS_CHANGED_EVENT,
        handleChanged,
      );
    }
  });
});
