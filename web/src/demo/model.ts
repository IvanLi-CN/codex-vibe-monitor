import { publishDemoRealtime } from "./events";
import type { DemoScene } from "./runtime";

export type DemoAction = {
  id: number;
  label: string;
  at: string;
};

type DemoState = {
  scene: DemoScene;
  revision: number;
  actions: DemoAction[];
  settings: Record<string, unknown>;
  externalApiKeys: Array<Record<string, unknown>>;
  accounts: Array<Record<string, unknown>>;
};

type DemoListener = () => void;

const DEMO_NOW = "2026-07-10T09:30:00.000Z";

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

const SENSITIVE_FIELD =
  /api[-_]?key|authorization|cookie|credential|oauth|password|secret|session|token/i;

function safePayload(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(safePayload);
  if (typeof value !== "object" || value === null) return value;
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .filter(([key]) => !SENSITIVE_FIELD.test(key))
      .map(([key, nested]) => [key, safePayload(nested)]),
  );
}

function createSettings() {
  return {
    proxy: {
      hijackEnabled: true,
      mergeUpstreamEnabled: true,
      fastModeRewriteMode: "disabled",
      upstream429MaxRetries: 3,
      websocketEnabled: true,
      upstreamWebsocketDefaultEnabled: true,
      requestBodyLoggingEnabled: true,
      responseBodyLoggingEnabled: true,
      encryptedSessionOwnerRoutingEnabled: false,
      defaultHijackEnabled: false,
      models: ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.4-mini"],
      enabledModels: ["gpt-5.6-sol", "gpt-5.6-terra"],
    },
    forwardProxy: {
      proxyUrls: ["socks5://demo-proxy.invalid:1080"],
      subscriptionUrls: ["https://demo.invalid/subscription"],
      subscriptionUpdateIntervalSecs: 3600,
      nodes: [
        {
          key: "demo-tokyo",
          source: "manual",
          displayName: "Tokyo demo relay",
          endpointUrl: "socks5://demo-proxy.invalid:1080",
          weight: 0.92,
          penalized: false,
          stats: {
            oneMinute: { attempts: 18, successRate: 0.97, avgLatencyMs: 184 },
            fifteenMinutes: { attempts: 254, successRate: 0.95, avgLatencyMs: 202 },
            oneHour: { attempts: 1038, successRate: 0.94, avgLatencyMs: 219 },
            oneDay: { attempts: 19680, successRate: 0.93, avgLatencyMs: 244 },
            sevenDays: { attempts: 137720, successRate: 0.94, avgLatencyMs: 238 },
          },
        },
      ],
    },
    pricing: {
      catalogVersion: "demo-2026-07",
      entries: [
        {
          model: "gpt-5.6-sol",
          inputPer1m: 5,
          outputPer1m: 30,
          cacheInputPer1m: 0.5,
          cacheReadPer1m: 0.5,
          cacheWritePer1m: 6.25,
          reasoningPer1m: null,
          source: "demo",
        },
        {
          model: "gpt-5.6-terra",
          inputPer1m: 2.5,
          outputPer1m: 15,
          cacheInputPer1m: 0.25,
          cacheReadPer1m: 0.25,
          cacheWritePer1m: 3.125,
          reasoningPer1m: null,
          source: "demo",
        },
      ],
    },
  };
}

function createAccounts(scene: DemoScene = "operational") {
  const attention = scene === "attention";
  return [
    {
      id: 101,
      kind: "oauth_codex",
      provider: "openai",
      displayName: "alpha@demo.invalid",
      email: "alpha@demo.invalid",
      accountId: "demo-alpha",
      groupName: "production",
      status: "active",
      displayStatus: "active",
      healthStatus: "normal",
      planType: "team",
      tags: [{ id: 1, name: "primary" }],
      boundProxyKeys: ["demo-tokyo"],
      createdAt: "2026-07-01T02:00:00Z",
      updatedAt: DEMO_NOW,
      primaryWindow: {
        usedPercent: 38,
        usedText: "38%",
        limitText: "weekly",
        windowDurationMins: 10080,
      },
      secondaryWindow: {
        usedPercent: 12,
        usedText: "12%",
        limitText: "5-hour",
        windowDurationMins: 300,
      },
    },
    {
      id: 102,
      kind: "api_key",
      provider: "openai",
      displayName: "backup-key",
      email: null,
      accountId: null,
      groupName: "standby",
      status: attention ? "error" : "active",
      displayStatus: attention ? "upstream_unavailable" : "active",
      healthStatus: attention ? "upstream_unavailable" : "normal",
      workStatus: attention ? "unavailable" : "idle",
      planType: "api",
      tags: [{ id: 2, name: "fallback" }],
      boundProxyKeys: [],
      lastError: attention ? "Simulated upstream timeout." : null,
      createdAt: "2026-07-02T02:00:00Z",
      updatedAt: DEMO_NOW,
      primaryWindow: {
        usedPercent: 82,
        usedText: "82%",
        limitText: "monthly",
        windowDurationMins: 43200,
      },
      secondaryWindow: null,
    },
  ];
}

function createState(scene: DemoScene): DemoState {
  return {
    scene,
    revision: 0,
    actions: [],
    settings: createSettings(),
    externalApiKeys: [
      {
        id: 41,
        name: "Demo integration",
        status: "active",
        prefix: "cvm_demo",
        lastUsedAt: "2026-07-10T09:00:00Z",
        createdAt: "2026-07-01T02:00:00Z",
        updatedAt: DEMO_NOW,
      },
    ],
    accounts: createAccounts(scene),
  };
}

class DemoModel {
  #state = createState("operational");
  #listeners = new Set<DemoListener>();

  get snapshot(): DemoState {
    return this.#state;
  }

  subscribe(listener: DemoListener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  setScene(scene: DemoScene) {
    if (this.#state.scene === scene) return;
    this.#state = createState(scene);
    this.#emit();
  }

  reset() {
    this.#state = createState(this.#state.scene);
    this.#emit();
  }

  record(label: string) {
    this.#state = {
      ...this.#state,
      revision: this.#state.revision + 1,
      actions: [
        { id: this.#state.revision + 1, label, at: DEMO_NOW },
        ...this.#state.actions,
      ].slice(0, 6),
    };
    this.#emit();
  }

  updateSettings(pathname: string, payload: unknown) {
    const nextPayload = safePayload(payload);
    const update =
      typeof nextPayload === "object" && nextPayload !== null
        ? (nextPayload as Record<string, unknown>)
        : {};
    const settings = clone(this.#state.settings);

    if (pathname === "/api/settings/proxy") {
      settings.proxy = { ...(settings.proxy as Record<string, unknown>), ...update };
    } else if (pathname === "/api/settings/forward-proxy") {
      settings.forwardProxy = { ...(settings.forwardProxy as Record<string, unknown>), ...update };
    } else if (pathname === "/api/settings/pricing") {
      settings.pricing = update;
    } else {
      Object.assign(settings, update);
    }

    this.#state = {
      ...this.#state,
      settings,
    };
    this.record("模拟保存配置");
    if (pathname === "/api/settings/proxy") return clone(settings.proxy);
    if (pathname === "/api/settings/forward-proxy") return clone(settings.forwardProxy);
    if (pathname === "/api/settings/pricing") return clone(settings.pricing);
    return clone(settings);
  }

  createAccount() {
    const account = {
      ...createAccounts(this.#state.scene)[0],
      id: 1000 + this.#state.accounts.length,
      displayName: `demo-account-${this.#state.accounts.length + 1}`,
      email: null,
      accountId: null,
      status: "active",
      groupName: "production",
      updatedAt: DEMO_NOW,
    };
    this.#state = { ...this.#state, accounts: [account, ...this.#state.accounts] };
    this.record("模拟创建账号");
    return clone(account);
  }

  createExternalApiKey() {
    const key = {
      id: 40 + this.#state.externalApiKeys.length + 1,
      name: `Demo integration ${this.#state.externalApiKeys.length + 1}`,
      status: "active",
      prefix: `cvm_demo_${this.#state.externalApiKeys.length + 1}`,
      lastUsedAt: null,
      createdAt: DEMO_NOW,
      updatedAt: DEMO_NOW,
    };
    this.#state = {
      ...this.#state,
      externalApiKeys: [key, ...this.#state.externalApiKeys],
    };
    this.record("模拟创建外部 API Key");
    return {
      key: clone(key),
      secret: "demo-generated-key-not-valid",
    };
  }

  injectLiveEvent() {
    const record = {
      ...createAccounts(this.#state.scene)[0],
      id: 9911,
      invokeId: "demo-live-event-9911",
      occurredAt: DEMO_NOW,
      createdAt: DEMO_NOW,
      source: "proxy",
      proxyDisplayName: "Tokyo demo relay",
      endpoint: "/v1/responses",
      model: "gpt-5.6-sol",
      status: "success",
      requestedServiceTier: "priority",
      serviceTier: "priority",
      inputTokens: 2300,
      outputTokens: 144,
      cacheInputTokens: 1980,
      totalTokens: 2444,
      cost: 0.0062,
      tUpstreamTtfbMs: 144,
      tTotalMs: 1088,
    };
    this.record("注入模拟实时事件");
    publishDemoRealtime({ type: "records", records: [record] });
  }

  #emit() {
    this.#listeners.forEach((listener) => {
      listener();
    });
  }
}

export const demoModel = new DemoModel();

export function demoNow() {
  return DEMO_NOW;
}
