/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../i18n";
import { ApiRequestError, type PoolRoutingSettings, type SettingsPayload } from "../lib/api";
import SettingsPage from "./Settings";

const hookMocks = vi.hoisted(() => ({
  useSettings: vi.fn(),
}));

const apiMocks = vi.hoisted(() => ({
  validateForwardProxyCandidate: vi.fn(),
}));

vi.mock("../hooks/useSettings", () => ({
  useSettings: hookMocks.useSettings,
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    validateForwardProxyCandidate: apiMocks.validateForwardProxyCandidate,
  };
});

vi.mock("../features/settings/ExternalApiKeysSettingsCard", () => ({
  ExternalApiKeysSettingsCard: () => <section data-testid="external-api-keys-settings-card" />,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function createSettingsPayload(): SettingsPayload {
  return {
    proxy: {
      hijackEnabled: false,
      mergeUpstreamEnabled: false,
      fastModeRewriteMode: "disabled",
      upstream429MaxRetries: 3,
      requestBodyLoggingEnabled: true,
      responseBodyLoggingEnabled: true,
      encryptedSessionOwnerRoutingEnabled: false,
      defaultHijackEnabled: false,
      models: ["gpt-5.6-sol"],
      enabledModels: ["gpt-5.6-sol"],
    },
    forwardProxy: {
      proxyUrls: ["http://proxy-order.example.com:8080"],
      subscriptionUrls: [],
      subscriptionUpdateIntervalSecs: 3600,
      nodes: [
        {
          key: "proxy-order",
          source: "manual",
          displayName: "proxy-order.example.com:8080",
          endpointUrl: "http://proxy-order.example.com:8080",
          weight: 0.75,
          penalized: false,
          stats: {
            oneMinute: { attempts: 1, successRate: 0.11, avgLatencyMs: 101 },
            fifteenMinutes: { attempts: 2, successRate: 0.22, avgLatencyMs: 202 },
            oneHour: { attempts: 3, successRate: 0.33, avgLatencyMs: 303 },
            oneDay: { attempts: 4, successRate: 0.44, avgLatencyMs: 404 },
            sevenDays: { attempts: 5, successRate: 0.55, avgLatencyMs: 505 },
          },
        },
      ],
    },
    pricing: {
      catalogVersion: "test",
      entries: [],
    },
  };
}

function createRoutingPayload(): PoolRoutingSettings {
  return {
    writesEnabled: true,
    apiKeyConfigured: true,
    maskedApiKey: "cvm-••••••••••••1234",
    maintenance: {
      primarySyncIntervalSecs: 300,
      secondarySyncIntervalSecs: 1800,
      priorityAvailableAccountCap: 100,
    },
    requestCompressionAlgorithm: "identity",
    requestCompressionLevelPreset: "balanced",
    timeouts: {
      responsesFirstByteTimeoutSecs: 120,
      compactFirstByteTimeoutSecs: 300,
      imageFirstByteTimeoutSecs: 300,
      responsesStreamTimeoutSecs: 300,
      compactStreamTimeoutSecs: 300,
    },
    cacheHitProtection: {
      enabled: false,
      lowHitRateThresholdPercent: 10,
      overflowMode: "queue",
      minimumInputTokens: 3840,
    },
  };
}

function renderSettingsPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);

  act(() => {
    root?.render(
      <I18nProvider>
        <SettingsPage />
      </I18nProvider>,
    );
  });
}

function expectTextInOrder(content: string, values: string[]) {
  let previousIndex = -1;
  for (const value of values) {
    const nextIndex = content.indexOf(value, previousIndex + 1);
    expect(nextIndex, `${value} should appear after index ${previousIndex}`).toBeGreaterThan(
      previousIndex,
    );
    previousIndex = nextIndex;
  }
}

describe("Settings forward proxy table", () => {
  beforeEach(() => {
    hookMocks.useSettings.mockReturnValue({
      settings: createSettingsPayload(),
      routing: createRoutingPayload(),
      isLoading: false,
      isProxySaving: false,
      isForwardProxySaving: false,
      isPricingSaving: false,
      isRoutingSaving: false,
      pricingRollbackVersion: 0,
      error: null,
      refresh: vi.fn(),
      saveProxy: vi.fn(),
      saveForwardProxy: vi.fn(),
      savePricing: vi.fn(),
      saveRouting: vi.fn(),
    });
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    hookMocks.useSettings.mockReset();
  });

  it("renders forward proxy desktop and mobile windows from longest to shortest window", () => {
    renderSettingsPage();

    const desktopTable = host?.querySelector(
      '[data-testid="settings-forward-proxy-desktop-table"]',
    );
    if (!(desktopTable instanceof HTMLTableElement)) {
      throw new Error("Missing forward proxy desktop table");
    }
    const desktopHeaders = Array.from(desktopTable.querySelectorAll("thead th")).map(
      (cell) => cell.textContent ?? "",
    );
    expect(desktopHeaders.slice(2, 7)).toEqual(["7 天", "1 天", "1 小时", "15 分钟", "1 分钟"]);

    const desktopCells = Array.from(desktopTable.querySelectorAll("tbody tr:first-child td")).map(
      (cell) => cell.textContent ?? "",
    );
    expect(desktopCells.slice(2, 7)).toEqual([
      "55.0%505 ms",
      "44.0%404 ms",
      "33.0%303 ms",
      "22.0%202 ms",
      "11.0%101 ms",
    ]);

    const mobileWindows = host?.querySelector(
      '[data-testid="settings-forward-proxy-mobile-windows"]',
    );
    if (!(mobileWindows instanceof HTMLElement)) {
      throw new Error("Missing forward proxy mobile windows");
    }
    expectTextInOrder(mobileWindows.textContent ?? "", [
      "7 天",
      "55.0%",
      "505 ms",
      "1 天",
      "44.0%",
      "404 ms",
      "1 小时",
      "33.0%",
      "303 ms",
      "15 分钟",
      "22.0%",
      "202 ms",
      "1 分钟",
      "11.0%",
      "101 ms",
    ]);
  });

  it("keeps validation error codes at the Settings state boundary", async () => {
    apiMocks.validateForwardProxyCandidate.mockRejectedValue(
      new ApiRequestError(422, "validation failed", { code: "invalid_xhttp_extra" }),
    );

    renderSettingsPage();

    const addButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) =>
      button.textContent?.includes("批量导入节点"),
    );
    if (!(addButton instanceof HTMLButtonElement)) {
      throw new Error("Missing batch proxy button");
    }
    act(() => {
      addButton.click();
    });

    const input = document.body.querySelector("#forward-proxy-modal-input");
    if (!(input instanceof HTMLTextAreaElement)) {
      throw new Error("Missing batch proxy input");
    }
    act(() => {
      const valueSetter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )?.set;
      valueSetter?.call(input, "vless://redacted@example.com:443");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const modal = input.closest("div.fixed");
    const validateButton = Array.from(modal?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "验证可用性",
    );
    if (!(validateButton instanceof HTMLButtonElement)) {
      throw new Error("Missing validation button");
    }
    await act(async () => {
      validateButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(apiMocks.validateForwardProxyCandidate).toHaveBeenCalledWith({
      kind: "proxyUrl",
      value: "vless://redacted@example.com:443",
    });
    expect(apiMocks.validateForwardProxyCandidate).toHaveBeenCalledTimes(12);
    expect(
      document.body.querySelector("tr[data-validation-code='invalid_xhttp_extra']"),
    ).not.toBeNull();
  });

  it("does not render a validated share link in the success message", async () => {
    const normalizedValue =
      "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?extra=opaque";
    apiMocks.validateForwardProxyCandidate.mockResolvedValue({
      ok: true,
      message: "XHTTP validation succeeded",
      normalizedValue,
      discoveredNodes: 1,
      latencyMs: 101,
    });

    renderSettingsPage();

    const addButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) =>
      button.textContent?.includes("批量导入节点"),
    );
    if (!(addButton instanceof HTMLButtonElement)) {
      throw new Error("Missing batch proxy button");
    }
    act(() => {
      addButton.click();
    });

    const input = document.body.querySelector("#forward-proxy-modal-input");
    if (!(input instanceof HTMLTextAreaElement)) {
      throw new Error("Missing batch proxy input");
    }
    act(() => {
      const valueSetter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value",
      )?.set;
      valueSetter?.call(input, normalizedValue);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const modal = input.closest("div.fixed");
    const validateButton = Array.from(modal?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "验证可用性",
    );
    if (!(validateButton instanceof HTMLButtonElement)) {
      throw new Error("Missing validation button");
    }
    await act(async () => {
      validateButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(document.body.textContent).not.toContain(normalizedValue);
    expect(document.body.textContent).toContain("可用 1 条，不可用 0 条");
  });

  it("renders independent request/response body logging switches", () => {
    renderSettingsPage();

    expect(host?.textContent).toContain("记录请求 body");
    expect(host?.textContent).toContain("记录响应 body");
  });

  it("persists the encrypted owner routing toggle through proxy settings", () => {
    const saveProxy = vi.fn();
    hookMocks.useSettings.mockReturnValue({
      settings: createSettingsPayload(),
      routing: createRoutingPayload(),
      isLoading: false,
      isProxySaving: false,
      isForwardProxySaving: false,
      isPricingSaving: false,
      isRoutingSaving: false,
      pricingRollbackVersion: 0,
      error: null,
      refresh: vi.fn(),
      saveProxy,
      saveForwardProxy: vi.fn(),
      savePricing: vi.fn(),
      saveRouting: vi.fn(),
    });

    renderSettingsPage();

    expect(host?.textContent).toContain("OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED");

    const toggle = host?.querySelector('button[aria-label="加密对话路由绑定"]');
    if (!(toggle instanceof HTMLButtonElement)) {
      throw new Error("Missing encrypted owner routing toggle");
    }
    act(() => {
      toggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(saveProxy).toHaveBeenCalledTimes(1);
    expect(saveProxy.mock.calls[0]?.[0]).toMatchObject({
      encryptedSessionOwnerRoutingEnabled: true,
    });
  });

  it("persists routing defaults through system settings", () => {
    const saveRouting = vi.fn();
    hookMocks.useSettings.mockReturnValue({
      settings: createSettingsPayload(),
      routing: createRoutingPayload(),
      isLoading: false,
      isProxySaving: false,
      isForwardProxySaving: false,
      isPricingSaving: false,
      isRoutingSaving: false,
      pricingRollbackVersion: 0,
      error: null,
      refresh: vi.fn(),
      saveProxy: vi.fn(),
      saveForwardProxy: vi.fn(),
      savePricing: vi.fn(),
      saveRouting,
    });

    renderSettingsPage();

    expect(host?.textContent).toContain("上游请求默认值");
    expect(host?.textContent).toContain("缓存命中保护");

    const priorityHandoffToggle = host?.querySelector('button[aria-label="优先级迁移准入控制"]');
    if (!(priorityHandoffToggle instanceof HTMLButtonElement)) {
      throw new Error("Missing priority handoff admission toggle");
    }
    act(() => {
      priorityHandoffToggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const cacheHitToggle = host?.querySelector('button[aria-label="缓存命中保护"]');
    if (!(cacheHitToggle instanceof HTMLButtonElement)) {
      throw new Error("Missing cache-hit protection toggle");
    }
    act(() => {
      cacheHitToggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const timeoutInput = host?.querySelector('input[name="responsesFirstByteTimeoutSecs"]');
    if (!(timeoutInput instanceof HTMLInputElement)) {
      throw new Error("Missing routing timeout input");
    }
    act(() => {
      const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      valueSetter?.call(timeoutInput, "180");
      timeoutInput.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const saveButton = Array.from(host?.querySelectorAll("button") ?? []).find((button) =>
      button.textContent?.includes("保存默认值"),
    );
    if (!(saveButton instanceof HTMLButtonElement)) {
      throw new Error("Missing routing save button");
    }
    expect(saveButton.disabled).toBe(false);
    act(() => {
      saveButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(saveRouting).toHaveBeenCalledTimes(1);
    expect(saveRouting).toHaveBeenCalledWith({
      availableModels: [],
      availableModelsMode: "denylist",
      requestCompressionAlgorithm: "identity",
      oauthRequestCompressionAlgorithm: "zstd",
      requestCompressionLevelPreset: "balanced",
      codexImagegenRewriteMode: "keep_original",
      priorityHandoffAdmissionEnabled: false,
      cacheHitProtection: {
        enabled: true,
        lowHitRateThresholdPercent: 10,
        overflowMode: "queue",
      },
      timeouts: {
        responsesFirstByteTimeoutSecs: 180,
        compactFirstByteTimeoutSecs: 300,
        imageFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
    });
  });
});
