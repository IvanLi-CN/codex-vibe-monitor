import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useLayoutEffect, useMemo, useRef, useState } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { SseReconnectReason, SseTerminalOutcome } from "../../lib/sse";
import AccountPoolLayout from "../../pages/account-pool/AccountPoolLayout";
import SystemLayout from "../../pages/system/SystemLayout";
import {
  AppLayout,
  type SseOfflineBannerStoryState,
  SseOfflineBannerStoryStateProvider,
} from "./AppLayout";

type StorybookSseState = {
  status: {
    phase: "connecting" | "reconnecting" | "disabled";
    downtimeMs: number;
    nextRetryAt: number | null;
    autoReconnect: boolean;
  };
  diagnostics: {
    attempt: number;
    reason: SseReconnectReason;
    activeTopics: string[];
    resumeTopics: string[];
    forcedSnapshotTopics: string[];
    lastMessageAgeMs: number | null;
    lastOpenAgeMs: number | null;
    lastErrorAgeMs: number | null;
    lastConnectionStartedAgeMs: number | null;
    lastTerminalOutcome: SseTerminalOutcome;
  };
};

function ageToTimestamp(now: number, ageMs: number | null) {
  return ageMs == null ? null : now - ageMs;
}

class MockEventSource implements EventTarget {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSED = 2;

  readonly url: string;
  readonly withCredentials = false;
  readyState = MockEventSource.CONNECTING;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
  onmessage: ((this: EventSource, ev: MessageEvent<string>) => unknown) | null = null;
  onopen: ((this: EventSource, ev: Event) => unknown) | null = null;

  #listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();

  constructor(url: string | URL) {
    this.url = typeof url === "string" ? url : url.toString();
    window.setTimeout(() => {
      if (this.readyState === MockEventSource.CLOSED) return;
      this.readyState = MockEventSource.OPEN;
      this.#emit("open", new Event("open"));
    }, 40);
  }

  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const bucket = this.#listeners.get(type) ?? new Set<EventListenerOrEventListenerObject>();
    bucket.add(listener);
    this.#listeners.set(type, bucket);
  }

  removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    this.#listeners.get(type)?.delete(listener);
  }

  dispatchEvent(event: Event) {
    this.#emit(event.type, event);
    return true;
  }

  close() {
    this.readyState = MockEventSource.CLOSED;
  }

  #emit(type: string, event: Event) {
    if (type === "open") this.onopen?.call(this as unknown as EventSource, event);
    if (type === "error") this.onerror?.call(this as unknown as EventSource, event);
    if (type === "message")
      this.onmessage?.call(this as unknown as EventSource, event as MessageEvent<string>);

    for (const listener of this.#listeners.get(type) ?? []) {
      if (typeof listener === "function") {
        listener(event);
      } else {
        listener.handleEvent(event);
      }
    }
  }
}

function MockPage({ title, description }: { title: string; description: string }) {
  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <span className="text-xs font-semibold uppercase tracking-[0.24em] text-primary/80">
            Site shell preview
          </span>
          <h2 className="section-title text-2xl">{title}</h2>
          <p className="section-description max-w-2xl">{description}</p>
        </div>
        <div className="grid gap-3 md:grid-cols-3">
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">Live SSE</p>
            <p className="mt-2 text-3xl font-semibold text-primary">Connected</p>
            <p className="mt-1 text-sm text-base-content/70">
              Header pulse, footer version, and nav all render together.
            </p>
          </div>
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">Backend version</p>
            <p className="mt-2 text-3xl font-semibold text-primary">v0.2.0</p>
            <p className="mt-1 text-sm text-base-content/70">
              Version endpoint is mocked for Storybook isolation.
            </p>
          </div>
          <div className="rounded-2xl border border-base-300 bg-base-100/90 p-4">
            <p className="text-sm font-medium text-base-content">Navigation</p>
            <p className="mt-2 text-3xl font-semibold text-primary">5 tabs</p>
            <p className="mt-1 text-sm text-base-content/70">
              Dashboard, stats, live, records, account pool, and system now share one shell.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function StorybookAppShellMock({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);

  useLayoutEffect(() => {
    originalFetchRef.current = window.fetch.bind(window);
    originalEventSourceRef.current = window.EventSource;

    window.fetch = async (input, init) => {
      const inputUrl =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const parsedUrl = new URL(inputUrl, window.location.origin);
      if (parsedUrl.pathname === "/api/version") {
        return new Response(JSON.stringify({ backend: "v0.2.0" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return (originalFetchRef.current ?? fetch)(input as RequestInfo | URL, init);
    };

    window.EventSource = MockEventSource as unknown as typeof EventSource;

    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
      if (originalEventSourceRef.current) {
        window.EventSource = originalEventSourceRef.current;
      }
    };
  }, []);

  return <>{children}</>;
}

function StorybookSseStateController({
  children,
  state,
}: {
  children: ReactNode;
  state?: StorybookSseState;
}) {
  const storyState = useMemo<SseOfflineBannerStoryState | undefined>(() => {
    if (!state) return undefined;

    const now = Date.now();
    return {
      status: state.status,
      diagnostics: {
        attempt: state.diagnostics.attempt,
        reason: state.diagnostics.reason,
        activeTopics: [...state.diagnostics.activeTopics],
        resumeTopics: [...state.diagnostics.resumeTopics],
        forcedSnapshotTopics: [...state.diagnostics.forcedSnapshotTopics],
        lastMessageAt: ageToTimestamp(now, state.diagnostics.lastMessageAgeMs),
        lastOpenAt: ageToTimestamp(now, state.diagnostics.lastOpenAgeMs),
        lastErrorAt: ageToTimestamp(now, state.diagnostics.lastErrorAgeMs),
        lastConnectionStartedAt: ageToTimestamp(now, state.diagnostics.lastConnectionStartedAgeMs),
        lastTerminalOutcome: state.diagnostics.lastTerminalOutcome,
      },
    };
  }, [state]);

  return (
    <SseOfflineBannerStoryStateProvider state={storyState}>
      {children}
    </SseOfflineBannerStoryStateProvider>
  );
}

function StorybookPwaRuntimeController({
  children,
  offline = false,
}: {
  children: ReactNode;
  offline?: boolean;
}) {
  const [ready, setReady] = useState(() => !offline);

  useLayoutEffect(() => {
    if (typeof window === "undefined") {
      return undefined;
    }

    if (!offline) {
      setReady(true);
      return undefined;
    }

    const hadOwnOnLine = Object.hasOwn(window.navigator, "onLine");
    const previousOnLineDescriptor = hadOwnOnLine
      ? Object.getOwnPropertyDescriptor(window.navigator, "onLine")
      : undefined;

    Object.defineProperty(window.navigator, "onLine", {
      configurable: true,
      get: () => false,
    });
    setReady(true);

    return () => {
      if (hadOwnOnLine && previousOnLineDescriptor) {
        Object.defineProperty(window.navigator, "onLine", previousOnLineDescriptor);
      } else {
        Reflect.deleteProperty(window.navigator, "onLine");
      }
      setReady(false);
    };
  }, [offline]);

  if (!ready) {
    return null;
  }

  return <>{children}</>;
}

const meta = {
  title: "Shell/Layout/App Layout",
  component: AppLayout,
  tags: ["autodocs"],
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <StorybookAppShellMock>
          <StorybookSseStateController
            state={context.parameters.sseState as StorybookSseState | undefined}
          >
            <StorybookPwaRuntimeController offline={Boolean(context.parameters.pwaOffline)}>
              <MemoryRouter
                initialEntries={[
                  context.parameters.initialEntry ?? "/account-pool/upstream-accounts",
                ]}
              >
                <Routes>
                  <Route path="/" element={<Story />}>
                    <Route
                      path="dashboard"
                      element={
                        <MockPage
                          title="Dashboard overview"
                          description="Global site layout preview with dashboard content mounted in the outlet."
                        />
                      }
                    />
                    <Route
                      path="stats"
                      element={
                        <MockPage
                          title="Stats workspace"
                          description="The same app shell can host time-series analytics and quota summaries."
                        />
                      }
                    />
                    <Route
                      path="live"
                      element={
                        <MockPage
                          title="Live monitor"
                          description="Realtime stream tables render inside the same site-wide shell."
                        />
                      }
                    />
                    <Route path="account-pool" element={<AccountPoolLayout />}>
                      <Route
                        path="upstream-accounts"
                        element={
                          <MockPage
                            title="Account Pool module active"
                            description="This story shows the whole site shell while the account-pool module is the active top-level tab."
                          />
                        }
                      />
                      <Route
                        path="groups"
                        element={
                          <MockPage
                            title="Account groups"
                            description="Mobile navigation can jump directly into grouped account operations without rendering a second nav row."
                          />
                        }
                      />
                      <Route
                        path="maintenance-records"
                        element={
                          <MockPage
                            title="Maintenance timeline"
                            description="Maintenance history stays reachable from the shared hamburger menu on compact screens."
                          />
                        }
                      />
                    </Route>
                    <Route path="system" element={<SystemLayout />}>
                      <Route
                        path="status"
                        element={
                          <MockPage
                            title="System workspace"
                            description="The top-level system workspace hosts status, tasks, shared settings, and forward proxy operations."
                          />
                        }
                      />
                      <Route
                        path="tasks"
                        element={
                          <MockPage
                            title="Task activity"
                            description="Task history remains reachable from the compact navigation drawer."
                          />
                        }
                      />
                      <Route
                        path="settings"
                        element={
                          <MockPage
                            title="System settings"
                            description="Shared settings become a first-class compact-navigation destination."
                          />
                        }
                      />
                      <Route
                        path="proxy"
                        element={
                          <MockPage
                            title="Proxy operations"
                            description="Forward proxy maintenance also routes through the unified mobile menu."
                          />
                        }
                      />
                    </Route>
                  </Route>
                </Routes>
              </MemoryRouter>
            </StorybookPwaRuntimeController>
          </StorybookSseStateController>
        </StorybookAppShellMock>
      </I18nProvider>
    ),
  ],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
  },
} satisfies Meta<typeof AppLayout>;

export default meta;

type Story = StoryObj<typeof meta>;

const offlineReconnectSseState = {
  status: {
    phase: "reconnecting",
    downtimeMs: 4 * 60 * 1000 + 39 * 1000,
    nextRetryAt: Date.now() + 8_000,
    autoReconnect: true,
  },
  diagnostics: {
    attempt: 26,
    reason: "manual",
    activeTopics: [
      "overview?t=day",
      "overview-counters?t=day",
      "timeseries?t=day",
      "model-breakdown?t=day",
      "tokens-breakdown?t=day",
      "cost-breakdown?t=day",
      "requests-breakdown?t=day",
      "latency-breakdown?t=day",
    ],
    resumeTopics: [],
    forcedSnapshotTopics: [
      "overview?t=day",
      "overview-counters?t=day",
      "timeseries?t=day",
      "model-breakdown?t=day",
      "tokens-breakdown?t=day",
      "cost-breakdown?t=day",
      "requests-breakdown?t=day",
      "latency-breakdown?t=day",
    ],
    lastMessageAgeMs: 4 * 60 * 1000 + 39 * 1000,
    lastOpenAgeMs: 5 * 60 * 1000,
    lastErrorAgeMs: 2_000,
    lastConnectionStartedAgeMs: 1_000,
    lastTerminalOutcome: "eventsource-error",
  },
} satisfies StorybookSseState;

async function assertOfflineReconnectBanner(canvasElement: HTMLElement) {
  const canvas = within(canvasElement.ownerDocument.body);
  await expect(canvas.getByText("实时连接已中断")).toBeVisible();
  await expect(canvas.getByTestId("app-sse-downtime")).toHaveTextContent("已掉线 4分39秒");
  await expect(canvas.getByRole("button", { name: "立即重连" })).toBeVisible();
  await expect(canvas.getByTestId("app-sse-diagnostics")).toBeVisible();
}

export const Default: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("app-header-inner")).toBeVisible();
    await expect(canvas.getByTestId("app-main")).toBeVisible();
    await expect(canvas.getByTestId("app-footer-inner")).toBeVisible();
    await expect(canvas.getByRole("link", { name: "号池" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await expect(canvas.queryByRole("button", { name: /打开导航菜单/i })).not.toBeInTheDocument();
  },
};

export const MobileNavigationMenu: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /打开导航菜单/i }));

    const documentScope = within(canvasElement.ownerDocument.body);
    await expect(documentScope.getByRole("link", { name: "维护记录" })).toBeVisible();
    await expect(documentScope.getByRole("link", { name: "状态" })).toBeVisible();

    await userEvent.click(documentScope.getByRole("link", { name: "维护记录" }));
    await expect(canvas.getByRole("heading", { name: "Maintenance timeline" })).toBeVisible();
  },
};

export const TabletNavigationMenu: Story = {
  parameters: {
    viewport: { defaultViewport: "tablet768" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: /打开导航菜单/i }));

    const documentScope = within(canvasElement.ownerDocument.body);
    await expect(documentScope.getByRole("link", { name: "设置" })).toBeVisible();
    await userEvent.click(documentScope.getByRole("link", { name: "设置" }));
    await expect(canvas.getByRole("heading", { name: "System settings" })).toBeVisible();
  },
};

export const OfflineReconnectBanner: Story = {
  parameters: {
    initialEntry: "/dashboard",
    sseState: offlineReconnectSseState,
  },
  play: async ({ canvasElement }) => assertOfflineReconnectBanner(canvasElement),
};

export const OfflineReconnectBannerMobile: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    initialEntry: "/dashboard",
    sseState: offlineReconnectSseState,
  },
  play: async (context) => {
    await assertOfflineReconnectBanner(context.canvasElement);

    const canvas = within(context.canvasElement);
    await expect(canvas.getByTestId("app-sse-reconnect-mobile-row")).toHaveClass("justify-end");
  },
};

export const PwaOfflineBannerDark: Story = {
  globals: {
    themeMode: "dark",
  },
  parameters: {
    initialEntry: "/dashboard",
    pwaOffline: true,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement.ownerDocument.body);
    await expect(canvas.getByTestId("pwa-offline-banner")).toBeVisible();
    await expect(canvas.getByText(/Offline app shell|离线应用壳层/)).toBeVisible();
    await expect(canvas.getByText(/Offline shell pending|离线壳待完成/)).toBeVisible();
  },
};
