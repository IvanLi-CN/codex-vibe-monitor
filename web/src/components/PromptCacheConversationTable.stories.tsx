import { useEffect, useRef, type ReactNode } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { MemoryRouter } from "react-router-dom";
import { I18nProvider } from "../i18n";
import type {
  PromptCacheConversationsResponse,
  UpstreamAccountDetail,
} from "../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

function jsonResponse(payload: unknown, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "Content-Type": "application/json",
    },
  });
}

function buildAccountDetail(
  id: number,
  displayName: string,
  overrides?: Partial<UpstreamAccountDetail>,
): UpstreamAccountDetail {
  return {
    id,
    kind: "oauth_codex",
    provider: "openai",
    displayName,
    groupName: "storybook-group",
    isMother: false,
    status: "active",
    enabled: true,
    email: `${displayName.toLowerCase().replace(/\s+/g, "-")}@example.com`,
    chatgptAccountId: `org_${id}`,
    chatgptUserId: `user_${id}`,
    planType: "team",
    maskedApiKey: null,
    lastSyncedAt: "2026-03-03T12:40:00.000Z",
    lastSuccessfulSyncAt: "2026-03-03T12:38:00.000Z",
    lastActivityAt: "2026-03-03T12:44:10.000Z",
    lastError: null,
    lastErrorAt: null,
    tokenExpiresAt: "2026-03-03T18:00:00.000Z",
    lastRefreshedAt: "2026-03-03T12:39:00.000Z",
    primaryWindow: {
      usedPercent: 22,
      usedText: "22 / 100",
      limitText: "100 requests",
      resetsAt: "2026-03-03T18:00:00.000Z",
      windowDurationMins: 300,
    },
    secondaryWindow: {
      usedPercent: 38,
      usedText: "38 / 100",
      limitText: "100 requests",
      resetsAt: "2026-03-10T00:00:00.000Z",
      windowDurationMins: 10080,
    },
    credits: null,
    localLimits: null,
    duplicateInfo: null,
    tags: [],
    effectiveRoutingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    },
    note: null,
    upstreamBaseUrl: null,
    history: [],
    ...overrides,
  };
}

const accountDetails = new Map<number, UpstreamAccountDetail>([
  [11, buildAccountDetail(11, "Pool Alpha", { isMother: true })],
  [12, buildAccountDetail(12, "Pool Beta")],
  [21, buildAccountDetail(21, "Pool Delta")],
  [22, buildAccountDetail(22, "Pool Epsilon")],
  [31, buildAccountDetail(31, "Pool Low")],
  [41, buildAccountDetail(41, "Pool High")],
]);

function StorybookPromptCacheAccountMock({
  children,
}: {
  children: ReactNode;
}) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const installedRef = useRef(false);

  if (typeof window !== "undefined" && !installedRef.current) {
    installedRef.current = true;
    originalFetchRef.current = window.fetch.bind(window);
    window.fetch = async (input, init) => {
      const method = (
        init?.method ||
        (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      const inputUrl =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      const parsedUrl = new URL(inputUrl, window.location.origin);
      const match = parsedUrl.pathname.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);

      if (match && method === "GET") {
        const accountId = Number(match[1]);
        const detail = accountDetails.get(accountId);
        if (!detail) {
          return jsonResponse({ message: "Not found" }, 404);
        }
        return jsonResponse(detail);
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init);
    };
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

const stats: PromptCacheConversationsResponse = {
  rangeStart: "2026-03-02T00:00:00.000Z",
  rangeEnd: "2026-03-03T00:00:00.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: "pck-chat-20260303-01",
      requestCount: 41,
      totalTokens: 56124,
      totalCost: 1.2842,
      createdAt: "2026-02-24T03:26:11.000Z",
      lastActivityAt: "2026-03-03T12:44:10.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 11,
          upstreamAccountName: "Pool Alpha",
          requestCount: 19,
          totalTokens: 26480,
          totalCost: 0.6124,
          lastActivityAt: "2026-03-03T12:44:10.000Z",
        },
        {
          upstreamAccountId: 12,
          upstreamAccountName: "Pool Beta",
          requestCount: 12,
          totalTokens: 17820,
          totalCost: 0.4018,
          lastActivityAt: "2026-03-03T11:21:00.000Z",
        },
        {
          upstreamAccountId: null,
          upstreamAccountName: null,
          requestCount: 10,
          totalTokens: 11824,
          totalCost: 0.27,
          lastActivityAt: "2026-03-03T10:00:00.000Z",
        },
        {
          upstreamAccountId: 13,
          upstreamAccountName: "Pool Hidden",
          requestCount: 3,
          totalTokens: 500,
          totalCost: 0.01,
          lastActivityAt: "2026-03-02T08:00:00.000Z",
        },
      ],
      last24hRequests: [
        {
          occurredAt: "2026-03-02T13:00:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 980,
          cumulativeTokens: 980,
        },
        {
          occurredAt: "2026-03-02T15:12:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1210,
          cumulativeTokens: 2190,
        },
        {
          occurredAt: "2026-03-02T17:53:00.000Z",
          status: "upstream_stream_error",
          isSuccess: false,
          requestTokens: 670,
          cumulativeTokens: 2860,
        },
        {
          occurredAt: "2026-03-02T20:40:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1460,
          cumulativeTokens: 4320,
        },
        {
          occurredAt: "2026-03-03T10:44:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1184,
          cumulativeTokens: 5504,
        },
      ],
    },
    {
      promptCacheKey: "pck-chat-20260303-02",
      requestCount: 16,
      totalTokens: 18209,
      totalCost: 0.4628,
      createdAt: "2026-02-20T08:09:33.000Z",
      lastActivityAt: "2026-03-03T11:40:28.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 21,
          upstreamAccountName: "Pool Delta",
          requestCount: 9,
          totalTokens: 10120,
          totalCost: 0.2588,
          lastActivityAt: "2026-03-03T11:40:28.000Z",
        },
        {
          upstreamAccountId: 22,
          upstreamAccountName: "Pool Epsilon",
          requestCount: 7,
          totalTokens: 8089,
          totalCost: 0.204,
          lastActivityAt: "2026-03-03T09:30:00.000Z",
        },
      ],
      last24hRequests: [
        {
          occurredAt: "2026-03-02T14:16:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 742,
          cumulativeTokens: 742,
        },
        {
          occurredAt: "2026-03-02T14:51:00.000Z",
          status: "invalid_api_key",
          isSuccess: false,
          requestTokens: 56,
          cumulativeTokens: 798,
        },
        {
          occurredAt: "2026-03-02T18:05:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 930,
          cumulativeTokens: 1728,
        },
        {
          occurredAt: "2026-03-03T11:40:28.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 804,
          cumulativeTokens: 2532,
        },
      ],
    },
  ],
};

const sharedScaleStats: PromptCacheConversationsResponse = {
  rangeStart: "2026-03-02T00:00:00.000Z",
  rangeEnd: "2026-03-03T00:00:00.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: "pck-low-volume",
      requestCount: 3,
      totalTokens: 420,
      totalCost: 0.01,
      createdAt: "2026-03-02T03:00:00.000Z",
      lastActivityAt: "2026-03-02T05:00:00.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 31,
          upstreamAccountName: "Pool Low",
          requestCount: 3,
          totalTokens: 420,
          totalCost: 0.01,
          lastActivityAt: "2026-03-02T05:00:00.000Z",
        },
      ],
      last24hRequests: [
        {
          occurredAt: "2026-03-02T03:00:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 100,
          cumulativeTokens: 100,
        },
        {
          occurredAt: "2026-03-02T05:00:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 120,
          cumulativeTokens: 220,
        },
      ],
    },
    {
      promptCacheKey: "pck-high-volume",
      requestCount: 8,
      totalTokens: 8600,
      totalCost: 0.21,
      createdAt: "2026-03-02T02:30:00.000Z",
      lastActivityAt: "2026-03-02T23:40:00.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 41,
          upstreamAccountName: "Pool High",
          requestCount: 8,
          totalTokens: 8600,
          totalCost: 0.21,
          lastActivityAt: "2026-03-02T23:40:00.000Z",
        },
      ],
      last24hRequests: [
        {
          occurredAt: "2026-03-02T02:30:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1200,
          cumulativeTokens: 1200,
        },
        {
          occurredAt: "2026-03-02T09:10:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 1800,
          cumulativeTokens: 3000,
        },
        {
          occurredAt: "2026-03-02T18:50:00.000Z",
          status: "upstream_stream_error",
          isSuccess: false,
          requestTokens: 900,
          cumulativeTokens: 3900,
        },
        {
          occurredAt: "2026-03-02T23:40:00.000Z",
          status: "completed",
          isSuccess: true,
          requestTokens: 2200,
          cumulativeTokens: 6100,
        },
      ],
    },
  ],
};

const meta = {
  title: "Monitoring/PromptCacheConversationTable",
  component: PromptCacheConversationTable,
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <MemoryRouter>
        <I18nProvider>
          <StorybookPromptCacheAccountMock>
            <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
              <main className="mx-auto w-full max-w-[1200px] space-y-4">
                <h2 className="text-xl font-semibold">
                  Prompt Cache 对话统计（Storybook Mock）
                </h2>
                <Story />
              </main>
            </div>
          </StorybookPromptCacheAccountMock>
        </I18nProvider>
      </MemoryRouter>
    ),
  ],
} satisfies Meta<typeof PromptCacheConversationTable>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Populated: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
};

export const Empty: Story = {
  args: {
    stats: {
      rangeStart: stats.rangeStart,
      rangeEnd: stats.rangeEnd,
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      conversations: [],
    },
    isLoading: false,
    error: null,
  },
};

export const Loading: Story = {
  args: {
    stats: null,
    isLoading: true,
    error: null,
  },
};

export const ErrorState: Story = {
  args: {
    stats: null,
    isLoading: false,
    error: "Network error",
  },
};

export const SharedScaleComparison: Story = {
  args: {
    stats: sharedScaleStats,
    isLoading: false,
    error: null,
  },
};

export const TooltipEdgeDensity: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Hover or tap the final token segment to verify the shared tooltip flips inward near the right table edge without clipping.",
      },
    },
  },
};
