import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect, useRef } from "react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  EffectiveRoutingRule,
  ForwardProxyBindingNode,
  UpstreamAccountGroupSummary,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
} from "../../lib/api";
import AccountPoolLayout from "../../pages/account-pool/AccountPoolLayout";
import GroupsPage from "../../pages/account-pool/Groups";
import type { UpstreamAccountsLocationState } from "../../pages/account-pool/UpstreamAccounts.shared-types";

type StoryScenario = "default" | "ungrouped-only" | "empty";

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
};

function buildAccount(
  id: number,
  overrides: Partial<UpstreamAccountSummary>,
): UpstreamAccountSummary {
  return {
    id,
    kind: "oauth_codex",
    provider: "openai",
    displayName: `Story account ${id}`,
    groupName: "production",
    isMother: false,
    status: "active",
    enabled: true,
    tags: [],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    ...overrides,
  };
}

function buildScenarioPayload(scenario: StoryScenario): UpstreamAccountListResponse {
  const forwardProxyNodes: ForwardProxyBindingNode[] = [
    {
      key: "__direct__",
      source: "direct",
      displayName: "Direct",
      protocolLabel: "DIRECT",
      penalized: false,
      selectable: true,
      last24h: [],
    },
    {
      key: "jp-edge-01",
      source: "manual",
      displayName: "JP Edge 01",
      protocolLabel: "HTTP",
      penalized: false,
      selectable: true,
      last24h: [],
    },
  ];

  const baseGroups: UpstreamAccountGroupSummary[] = [
    {
      groupName: "production",
      note: "Premium traffic group.",
      boundProxyKeys: ["__direct__", "jp-edge-01"],
      concurrencyLimit: 6,
      nodeShuntEnabled: true,
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 2,
      routingRule: {
        allowCutOut: false,
        allowCutIn: true,
        priorityTier: "no_new",
        fastModeRewriteMode: "force_add",
        concurrencyLimit: 6,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 3,
      },
    },
    {
      groupName: "staging",
      note: "Lower-risk staging traffic.",
      boundProxyKeys: ["jp-edge-01"],
      concurrencyLimit: 2,
      nodeShuntEnabled: false,
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
    },
  ];

  if (scenario === "empty") {
    return {
      writesEnabled: true,
      items: [],
      groups: [],
      forwardProxyNodes,
      hasUngroupedAccounts: false,
      total: 0,
      page: 1,
      pageSize: 20,
      metrics: {
        total: 0,
        oauth: 0,
        apiKey: 0,
        attention: 0,
      },
      routing: null,
    };
  }

  if (scenario === "ungrouped-only") {
    const items = [
      buildAccount(21, {
        displayName: "Ungrouped OAuth Alpha",
        groupName: null,
        planType: "team",
      }),
      buildAccount(22, {
        kind: "api_key_codex",
        displayName: "Ungrouped API Key Beta",
        groupName: null,
        planType: "local",
      }),
    ];
    return {
      writesEnabled: true,
      items,
      groups: [],
      forwardProxyNodes,
      hasUngroupedAccounts: true,
      total: items.length,
      page: 1,
      pageSize: items.length,
      metrics: {
        total: items.length,
        oauth: 1,
        apiKey: 1,
        attention: 0,
      },
      routing: null,
    };
  }

  const items = [
    buildAccount(1, {
      displayName: "Production Team 01",
      groupName: "production",
      planType: "team",
      isMother: true,
    }),
    buildAccount(2, {
      displayName: "Production Team 02",
      groupName: "production",
      planType: "team",
    }),
    buildAccount(3, {
      kind: "api_key_codex",
      displayName: "Production API 01",
      groupName: "production",
      planType: "local",
    }),
    buildAccount(4, {
      displayName: "Staging Free 01",
      groupName: "staging",
      planType: "free",
    }),
    buildAccount(5, {
      displayName: "Ungrouped Rescue",
      groupName: null,
      planType: "free",
    }),
  ];

  return {
    writesEnabled: true,
    items,
    groups: baseGroups,
    forwardProxyNodes,
    hasUngroupedAccounts: true,
    total: items.length,
    page: 1,
    pageSize: items.length,
    metrics: {
      total: items.length,
      oauth: 4,
      apiKey: 1,
      attention: 1,
    },
    routing: null,
  };
}

function jsonResponse(payload: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status,
      headers: { "Content-Type": "application/json" },
    }),
  );
}

function StorybookGroupsMock({
  scenario,
  children,
}: {
  scenario: StoryScenario;
  children: ReactNode;
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const installedRef = useRef(false);

  if (typeof window !== "undefined" && !installedRef.current) {
    installedRef.current = true;
    originalFetchRef.current = window.fetch.bind(window);

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (
        init?.method || (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      const inputUrl =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const url = new URL(inputUrl, window.location.origin);

      if (url.pathname === "/api/pool/upstream-accounts" && method === "GET") {
        return jsonResponse(buildScenarioPayload(scenario));
      }

      if (url.pathname === "/api/pool/forward-proxy-binding-nodes" && method === "GET") {
        return jsonResponse([]);
      }

      const groupMatch = url.pathname.match(/^\/api\/pool\/upstream-account-groups\/([^/]+)$/);
      if (groupMatch && method === "PUT") {
        const groupName = decodeURIComponent(groupMatch[1]);
        const existing =
          buildScenarioPayload(scenario).groups.find((group) => group.groupName === groupName) ??
          null;
        return jsonResponse(
          existing ?? {
            groupName,
            note: "",
            boundProxyKeys: [],
            concurrencyLimit: 0,
            nodeShuntEnabled: false,
            upstream429RetryEnabled: false,
            upstream429MaxRetries: 0,
          },
        );
      }

      return originalFetchRef.current
        ? originalFetchRef.current(input as Parameters<typeof fetch>[0], init)
        : fetch(input as Parameters<typeof fetch>[0], init);
    };

    window.fetch = mockedFetch;
  }

  useEffect(() => {
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
    };
  }, []);

  return <>{children}</>;
}

function UpstreamAccountsStateEcho() {
  const location = useLocation();
  const state = location.state as UpstreamAccountsLocationState | null;
  return (
    <div className="surface-panel">
      <div className="surface-panel-body">
        <p data-testid="groups-story-state">
          {state?.presetGroupFilter
            ? `${state.presetGroupFilter.mode}:${state.presetGroupFilter.query}`
            : "none"}
        </p>
      </div>
    </div>
  );
}

function GroupsPageRouter({ initialEntry = "/account-pool/groups" }: { initialEntry?: string }) {
  return (
    <MemoryRouter initialEntries={[initialEntry]}>
      <Routes>
        <Route path="/account-pool" element={<AccountPoolLayout />}>
          <Route path="groups" element={<GroupsPage />} />
          <Route path="upstream-accounts" element={<UpstreamAccountsStateEcho />} />
          <Route
            path="upstream-accounts/new"
            element={<div data-testid="groups-story-create-page">create</div>}
          />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}

const meta = {
  title: "Account Pool/Pages/Groups",
  component: GroupsPage,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <Story />
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof GroupsPage>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <StorybookGroupsMock scenario="default">
      <GroupsPageRouter />
    </StorybookGroupsMock>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.findByRole("heading", { name: "分组总览" })).resolves.toBeTruthy();
    await expect(canvas.findByText("production")).resolves.toBeTruthy();
    await expect(canvas.findByTestId("account-pool-groups-list")).resolves.toBeTruthy();

    const viewAccountsLink = await canvas.findByRole("link", {
      name: "查看上游账号",
    });
    await userEvent.click(viewAccountsLink);

    const updatedCanvas = within(canvasElement);
    await expect(updatedCanvas.findByTestId("groups-story-state")).resolves.toHaveTextContent(
      "exact:production",
    );
  },
};

export const UngroupedOnly: Story = {
  render: () => (
    <StorybookGroupsMock scenario="ungrouped-only">
      <GroupsPageRouter />
    </StorybookGroupsMock>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.findByText("未分组")).resolves.toBeTruthy();
    await expect(canvas.findByTestId("account-pool-group-row-ungrouped")).resolves.toBeTruthy();
  },
};

export const EmptyState: Story = {
  render: () => (
    <StorybookGroupsMock scenario="empty">
      <GroupsPageRouter />
    </StorybookGroupsMock>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.findByTestId("account-pool-groups-empty")).resolves.toBeTruthy();
    await expect(canvas.findByRole("link", { name: "创建上游账号" })).resolves.toBeTruthy();
  },
};
