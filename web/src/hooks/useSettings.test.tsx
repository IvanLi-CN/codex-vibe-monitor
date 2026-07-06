/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ForwardProxySettings,
  PricingSettings,
  ProxySettings,
  SettingsPayload,
} from "../lib/api";
import { UPSTREAM_ACCOUNTS_CHANGED_EVENT } from "../lib/upstreamAccountsEvents";
import { useSettings } from "./useSettings";

const apiMocks = vi.hoisted(() => ({
  fetchSettings: vi.fn<() => Promise<SettingsPayload>>(),
  updateProxySettings: vi.fn<(payload: {
    hijackEnabled: boolean;
    mergeUpstreamEnabled: boolean;
    fastModeRewriteMode?: "disabled" | "fill_missing" | "force_priority";
    upstream429MaxRetries: number;
    websocketEnabled: boolean;
    upstreamWebsocketDefaultEnabled: boolean;
    requestBodyLoggingEnabled: boolean;
    responseBodyLoggingEnabled: boolean;
    encryptedSessionOwnerRoutingEnabled: boolean;
    enabledModels: string[];
  }) => Promise<ProxySettings>>(),
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
    updateProxySettings: apiMocks.updateProxySettings,
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
    proxy: {
      hijackEnabled: false,
      mergeUpstreamEnabled: false,
      fastModeRewriteMode: "disabled",
      upstream429MaxRetries: 3,
      websocketEnabled: false,
      upstreamWebsocketDefaultEnabled: false,
      requestBodyLoggingEnabled: true,
      responseBodyLoggingEnabled: true,
      encryptedSessionOwnerRoutingEnabled: true,
      defaultHijackEnabled: false,
      models: ["gpt-5.4", "gpt-5.5", "gpt-5.5-pro"],
      enabledModels: ["gpt-5.4", "gpt-5.5", "gpt-5.5-pro"],
    },
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
  const { settings, error, isLoading, saveProxy, saveForwardProxy } = useSettings();

  return (
    <div>
      <div data-testid="loading">{String(isLoading)}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="proxy-enabled-models">
        {settings?.proxy.enabledModels.join(",") ?? ""}
      </div>
      <div data-testid="proxy-body-logging">
        {settings
          ? `${settings.proxy.requestBodyLoggingEnabled}/${settings.proxy.responseBodyLoggingEnabled}`
          : ""}
      </div>
      <div data-testid="proxy-encrypted-owner-routing">
        {settings ? String(settings.proxy.encryptedSessionOwnerRoutingEnabled) : ""}
      </div>
      <div data-testid="proxy-urls">
        {settings?.forwardProxy.proxyUrls.join(",") ?? ""}
      </div>
      <button
        data-testid="save-proxy"
        disabled={!settings}
        onClick={() => {
          if (!settings) return;
          void saveProxy({
            ...settings.proxy,
            hijackEnabled: true,
            mergeUpstreamEnabled: true,
            requestBodyLoggingEnabled: false,
            responseBodyLoggingEnabled: false,
            encryptedSessionOwnerRoutingEnabled: false,
            enabledModels: ["gpt-5.5", "gpt-5.5-pro"],
          });
        }}
      >
        save proxy
      </button>
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
  apiMocks.updateProxySettings.mockImplementation(async (payload) => ({
    hijackEnabled: payload.hijackEnabled,
    mergeUpstreamEnabled: payload.hijackEnabled
      ? payload.mergeUpstreamEnabled
      : false,
    fastModeRewriteMode: payload.fastModeRewriteMode ?? "disabled",
    upstream429MaxRetries: payload.upstream429MaxRetries,
    websocketEnabled: payload.websocketEnabled,
    upstreamWebsocketDefaultEnabled: payload.upstreamWebsocketDefaultEnabled,
    requestBodyLoggingEnabled: payload.requestBodyLoggingEnabled,
    responseBodyLoggingEnabled: payload.responseBodyLoggingEnabled,
    encryptedSessionOwnerRoutingEnabled: payload.encryptedSessionOwnerRoutingEnabled,
    defaultHijackEnabled: false,
    models: ["gpt-5.4", "gpt-5.5", "gpt-5.5-pro"],
    enabledModels: payload.enabledModels,
  }));
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
  it("saves proxy settings and rolls back when save fails", async () => {
    render(<Probe />);
    await flushAsync();

    expect(text("proxy-enabled-models")).toContain("gpt-5.4");

    click("save-proxy");
    await flushAsync();

    expect(apiMocks.updateProxySettings).toHaveBeenCalledTimes(1);
    expect(text("proxy-enabled-models")).toContain("gpt-5.5");
    expect(text("proxy-enabled-models")).toContain("gpt-5.5-pro");
    expect(text("proxy-body-logging")).toBe("false/false");
    expect(text("proxy-encrypted-owner-routing")).toBe("false");
    expect(apiMocks.updateProxySettings.mock.calls[0]?.[0]).toMatchObject({
      encryptedSessionOwnerRoutingEnabled: false,
    });
    expect(text("error")).toBe("");

    apiMocks.updateProxySettings.mockRejectedValueOnce(new Error("proxy save failed"));
    click("save-proxy");
    await flushAsync();

    expect(apiMocks.updateProxySettings).toHaveBeenCalledTimes(2);
    expect(text("error")).toBe("proxy save failed");
    expect(text("proxy-enabled-models")).toContain("gpt-5.5");
    expect(text("proxy-enabled-models")).toContain("gpt-5.5-pro");
  });

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
