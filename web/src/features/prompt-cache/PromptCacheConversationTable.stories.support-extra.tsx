import type { Meta, StoryObj } from "@storybook/react-vite";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { PromptCacheConversationsResponse } from "../../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";
import {
  bindingByPromptCacheKey,
  buildBindingResponse,
  buildInvocationRecord,
  buildPreviewFromRecord,
  CONVERSATION_LARGE_HISTORY_KEY,
  CONVERSATION_ONE_KEY,
  CONVERSATION_ROUTING_KEY,
  CONVERSATION_SHORT_KEY,
  largeHistory,
  largeHistoryPreviews,
  MockEventSource,
  operationEventsByPromptCacheKey,
  StorybookPromptCacheAccountMock,
  shortSameDayHistory,
  shortSameDayPreviews,
  stats,
} from "./PromptCacheConversationTable.stories.support-base";

const sharedScaleStats: PromptCacheConversationsResponse = {
  rangeStart: "2026-03-02T00:00:00.000Z",
  rangeEnd: "2026-03-03T00:00:00.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: "019d2b69-ca16-73f2-bf97-0e9b9a1f0c31",
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
      requestCount: 3,
      totalTokens: 420,
      totalCost: 0.01,
      createdAt: "2026-03-02T03:00:00.000Z",
      lastActivityAt: "2026-03-02T05:00:00.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 31,
          upstreamAccountName: "sweep.q1h2@watch.example",
          requestCount: 3,
          totalTokens: 420,
          totalCost: 0.01,
          lastActivityAt: "2026-03-02T05:00:00.000Z",
        },
      ],
      recentInvocations: [
        buildPreviewFromRecord(
          buildInvocationRecord({
            id: 701,
            invokeId: "invoke-low-01",
            promptCacheKey: "019d2b69-ca16-73f2-bf97-0e9b9a1f0c31",
            occurredAt: "2026-03-02T05:00:00.000Z",
            totalTokens: 120,
            cost: 0.003,
            proxyDisplayName: "hong-kong-edge-01",
            upstreamAccountId: 31,
            upstreamAccountName: "sweep.q1h2@watch.example",
          }),
        ),
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
      promptCacheKey: "019d2b77-b081-7180-80bd-5cc31df7f9b4",
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
      requestCount: 8,
      totalTokens: 8600,
      totalCost: 0.21,
      createdAt: "2026-03-02T02:30:00.000Z",
      lastActivityAt: "2026-03-02T23:40:00.000Z",
      upstreamAccounts: [
        {
          upstreamAccountId: 41,
          upstreamAccountName: "burst.f9m4@watch.example",
          requestCount: 8,
          totalTokens: 8600,
          totalCost: 0.21,
          lastActivityAt: "2026-03-02T23:40:00.000Z",
        },
      ],
      recentInvocations: [
        buildPreviewFromRecord(
          buildInvocationRecord({
            id: 801,
            invokeId: "invoke-high-01",
            promptCacheKey: "019d2b77-b081-7180-80bd-5cc31df7f9b4",
            occurredAt: "2026-03-02T23:40:00.000Z",
            totalTokens: 2200,
            cost: 0.052,
            proxyDisplayName: "london-edge-02",
            upstreamAccountId: 41,
            upstreamAccountName: "burst.f9m4@watch.example",
          }),
        ),
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

const shortSameDayStats: PromptCacheConversationsResponse = {
  rangeStart: "2026-05-13T16:00:00.000Z",
  rangeEnd: "2026-05-14T15:59:59.000Z",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: CONVERSATION_SHORT_KEY,
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 21,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "CIII",
      requestCount: shortSameDayHistory.length,
      totalTokens: shortSameDayHistory.reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
      totalCost: shortSameDayHistory.reduce((sum, record) => sum + (record.cost ?? 0), 0),
      createdAt: shortSameDayHistory.at(-1)?.occurredAt ?? "",
      lastActivityAt: shortSameDayHistory[0]?.occurredAt ?? "",
      upstreamAccounts: [
        {
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
          requestCount: shortSameDayHistory.filter((record) => record.upstreamAccountId === 21)
            .length,
          totalTokens: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 21)
            .reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
          totalCost: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 21)
            .reduce((sum, record) => sum + (record.cost ?? 0), 0),
          lastActivityAt: shortSameDayHistory[0]?.occurredAt ?? "",
        },
        {
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
          requestCount: shortSameDayHistory.filter((record) => record.upstreamAccountId === 22)
            .length,
          totalTokens: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 22)
            .reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
          totalCost: shortSameDayHistory
            .filter((record) => record.upstreamAccountId === 22)
            .reduce((sum, record) => sum + (record.cost ?? 0), 0),
          lastActivityAt:
            shortSameDayHistory.find((record) => record.upstreamAccountId === 22)?.occurredAt ?? "",
        },
      ],
      recentInvocations: shortSameDayPreviews,
      last24hRequests: shortSameDayHistory
        .slice()
        .reverse()
        .map((record, index, records) => ({
          occurredAt: record.occurredAt,
          status: record.status ?? "completed",
          isSuccess: record.failureClass === "none",
          requestTokens: record.totalTokens ?? 0,
          cumulativeTokens: records
            .slice(0, index + 1)
            .reduce((sum, item) => sum + (item.totalTokens ?? 0), 0),
        })),
    },
  ],
};

const routingStoryStats: PromptCacheConversationsResponse = {
  ...shortSameDayStats,
  conversations: shortSameDayStats.conversations.map((conversation) => ({
    ...conversation,
    promptCacheKey: CONVERSATION_ROUTING_KEY,
  })),
};

const largeHistoryStats: PromptCacheConversationsResponse = {
  rangeStart: largeHistory.at(-1)?.occurredAt ?? "",
  rangeEnd: largeHistory[0]?.occurredAt ?? "",
  selectionMode: "count",
  selectedLimit: 50,
  selectedActivityHours: null,
  implicitFilter: { kind: null, filteredCount: 0 },
  conversations: [
    {
      promptCacheKey: CONVERSATION_LARGE_HISTORY_KEY,
      hasEncryptedSessionOwner: false,
      encryptedOwnerAccountId: null,
      encryptedOwnerAccountName: null,
      encryptedOwnerGroupName: null,
      requestCount: largeHistory.length,
      totalTokens: largeHistory.reduce((sum, record) => sum + (record.totalTokens ?? 0), 0),
      totalCost: largeHistory.reduce((sum, record) => sum + (record.cost ?? 0), 0),
      createdAt: largeHistory.at(-1)?.occurredAt ?? "",
      lastActivityAt: largeHistory[0]?.occurredAt ?? "",
      upstreamAccounts: [
        {
          upstreamAccountId: 11,
          upstreamAccountName: "growth.6vv4@relay.example",
          requestCount: Math.ceil(largeHistory.length / 2),
          totalTokens: 1_384_000_000,
          totalCost: 910.24,
          lastActivityAt: largeHistory[0]?.occurredAt ?? "",
        },
        {
          upstreamAccountId: 12,
          upstreamAccountName: "backup.f3x2@ops.example",
          requestCount: Math.floor(largeHistory.length / 2),
          totalTokens: 1_216_000_000,
          totalCost: 801.18,
          lastActivityAt: largeHistory[1]?.occurredAt ?? "",
        },
      ],
      recentInvocations: largeHistoryPreviews,
      last24hRequests: largeHistory
        .slice(0, 120)
        .reverse()
        .map((record, index, records) => ({
          occurredAt: record.occurredAt,
          status: record.status ?? "completed",
          isSuccess: record.failureClass === "none",
          requestTokens: record.totalTokens ?? 0,
          cumulativeTokens: records
            .slice(0, index + 1)
            .reduce((sum, item) => sum + (item.totalTokens ?? 0), 0),
        })),
    },
  ],
};

const meta = {
  title: "Monitoring/PromptCacheConversationTable",
  component: PromptCacheConversationTable,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <MemoryRouter>
        <I18nProvider>
          <StorybookPromptCacheAccountMock>
            <div className="min-h-screen bg-base-200 px-4 py-6 text-base-content sm:px-6">
              <main className="app-shell-boundary space-y-4">
                <h2 className="text-xl font-semibold">对话</h2>
                <Story />
              </main>
            </div>
          </StorybookPromptCacheAccountMock>
        </I18nProvider>
      </MemoryRouter>
    ),
  ],
} satisfies Meta<typeof PromptCacheConversationTable>;

type Story = StoryObj<typeof meta>;

const liveSyncSettledStats: PromptCacheConversationsResponse = {
  ...stats,
  conversations: stats.conversations.map((conversation, index) =>
    index !== 0
      ? conversation
      : {
          ...conversation,
          lastActivityAt: "2026-03-27T03:15:19.000Z",
          recentInvocations: [
            buildPreviewFromRecord(
              buildInvocationRecord({
                id: 507,
                invokeId: "invoke-pck-01-live-sync",
                promptCacheKey: CONVERSATION_ONE_KEY,
                occurredAt: "2026-03-27T03:15:19.000Z",
                upstreamAccountId: 11,
                upstreamAccountName: "growth.6vv4@relay.example",
                proxyDisplayName: "tokyo-edge-live-sync",
                totalTokens: 70214,
                inputTokens: 64810,
                cacheInputTokens: 61504,
                outputTokens: 5404,
                reasoningTokens: 924,
                reasoningEffort: "high",
                cost: 0.0468,
                responseContentEncoding: "gzip, br",
                tUpstreamConnectMs: 544,
                tUpstreamTtfbMs: 118,
                tUpstreamStreamMs: 680,
                tTotalMs: 1436,
              }),
            ),
            ...conversation.recentInvocations.slice(0, 4),
          ],
          last24hRequests: [
            ...conversation.last24hRequests.slice(0, -1),
            {
              occurredAt: "2026-03-27T03:15:19.000Z",
              status: "completed",
              isSuccess: true,
              requestTokens: 70214,
              cumulativeTokens: 854323,
            },
          ],
        },
  ),
};

function seedDrawerBindingAndTimeoutsStory() {
  bindingByPromptCacheKey.set(
    CONVERSATION_SHORT_KEY,
    buildBindingResponse({
      promptCacheKey: CONVERSATION_SHORT_KEY,
      bindingKind: "upstreamAccount",
      upstreamAccountId: 21,
      upstreamAccountName: "growth.6vv4@relay.example",
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 21,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "CIII",
      timeouts: {
        responsesFirstByteTimeoutSecs: 40,
        compactFirstByteTimeoutSecs: 180,
        responsesStreamTimeoutSecs: 225,
        compactStreamTimeoutSecs: 300,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "conversation",
        compactFirstByteTimeoutSecs: "account",
        responsesStreamTimeoutSecs: "conversation",
        compactStreamTimeoutSecs: "root",
      },
      allowSwitchUpstream: true,
      fastModeRewriteMode: "force_add",
      imageToolRewriteMode: "force_remove",
      codexImagegenRewriteMode: "force_add",
      availableModels: ["gpt-5.1-codex-max", "gpt-5.1-codex-mini"],
      forwardProxyKey: "__direct__",
      forwardProxyKeys: ["__direct__", "tokyo-edge-01"],
      policyFieldSources: {
        allowSwitchUpstream: "conversation",
        fastModeRewriteMode: "conversation",
        imageToolRewriteMode: "conversation",
        codexImagegenRewriteMode: "conversation",
        availableModels: "conversation",
        forwardProxyKey: "conversation",
      },
      updatedAt: "2026-05-13T23:42:00.000Z",
    }),
  );
}

async function assertDrawerBindingAndTimeoutsStory(canvasElement: HTMLElement) {
  const documentScope = within(canvasElement.ownerDocument.body);
  const historyButton = documentScope.getAllByRole("button", {
    name: /打开全部调用记录|open full call history/i,
  })[0];

  await userEvent.click(historyButton);
  await userEvent.click(await documentScope.findByRole("tab", { name: /路由|Routing/i }));
  await waitFor(() => {
    expect(
      Array.from(MockEventSource.instances).some(
        (instance) => instance.readyState === MockEventSource.OPEN,
      ),
    ).toBe(true);
  });
  const descriptor = {
    topic: "prompt-cache.conversation-binding.current",
    params: { promptCacheKey: CONVERSATION_SHORT_KEY },
  };
  MockEventSource.emitMessage({
    type: "snapshot",
    topic: descriptor,
    topicKey: JSON.stringify(descriptor),
    schemaEpoch: "prompt-cache.conversation-binding.current/v1",
    cursor: 1,
    payload: bindingByPromptCacheKey.get(CONVERSATION_SHORT_KEY),
  });
  const drawerShell = canvasElement.ownerDocument.body.querySelector(".drawer-shell");
  await expect(drawerShell).toHaveClass("drawer-shell--detail-wide");
  await expect(drawerShell?.parentElement).toHaveClass("drawer-frame");
  const viewportWidth = canvasElement.ownerDocument.defaultView?.innerWidth ?? 0;
  const expectedWidth = Math.min(90 * 16, viewportWidth - 32);
  const actualWidth = drawerShell?.getBoundingClientRect().width ?? 0;
  await expect(Math.abs(actualWidth - expectedWidth)).toBeLessThanOrEqual(1);
  await expect(await documentScope.findByText(/路由绑定|Route binding/i)).toBeInTheDocument();
  await userEvent.click(await documentScope.findByRole("tab", { name: /设置|Settings/i }));
  await expect(documentScope.getByText(/当前对话覆盖|Conversation overrides/i)).toBeInTheDocument();
  await expect(
    documentScope.getAllByText(/允许换上游|Allow switching upstream/i).length,
  ).toBeGreaterThan(0);
  await expect(documentScope.getAllByText(/强制添加|Force add/i).length).toBeGreaterThan(0);
  await expect(documentScope.getAllByText(/强制移除|Force remove/i).length).toBeGreaterThan(0);
  await expect(documentScope.getAllByText(/Codex imagegen/i).length).toBeGreaterThan(0);
  await expect(documentScope.getAllByText(/对话|Conversation/i).length).toBeGreaterThan(0);
  await expect(documentScope.getAllByText(/gpt-5\.1-codex-max/i).length).toBeGreaterThan(0);
  await expect(documentScope.getAllByText(/gpt-5\.1-codex-mini/i).length).toBeGreaterThan(0);
  await expect(documentScope.getByText(/40s/)).toBeInTheDocument();
  await expect(documentScope.getAllByText(/对话|Conversation/i).length).toBeGreaterThan(0);
  await expect(documentScope.queryByText(/优先级|Priority/i)).not.toBeInTheDocument();
  await expect(documentScope.queryByText(/切入|Cut in/i)).not.toBeInTheDocument();

  const imageToolHelp = documentScope
    .getAllByRole("button", { name: /图片工具 help|Image tool help/i })
    .find((button) => button.closest(".border-t") != null);
  if (!imageToolHelp) {
    throw new Error("missing expanded image tool help");
  }
  await userEvent.click(imageToolHelp);
  await expect(documentScope.getByText(/Codex Full 与 Lite imagegen 请单独配置/i)).toBeVisible();

  await expect(
    documentScope.getByRole("button", {
      name: /清除对话覆盖: FAST 模式|Clear conversation override: FAST mode/i,
    }),
  ).toBeInTheDocument();
  await expect(
    documentScope.getByRole("button", {
      name: /清除对话覆盖: 图片工具|Clear conversation override: Image tool/i,
    }),
  ).toBeInTheDocument();
}

export async function playDrawerBindingAndTimeoutsStory(canvasElement: HTMLElement) {
  seedDrawerBindingAndTimeoutsStory();
  await assertDrawerBindingAndTimeoutsStory(canvasElement);
}

async function prepareDrawerOperationsStory(canvasElement: HTMLElement) {
  const documentScope = within(canvasElement.ownerDocument.body);
  const historyButton = documentScope.getAllByRole("button", {
    name: /打开全部调用记录|open full call history/i,
  })[0];

  await userEvent.click(historyButton);
  await userEvent.click(await documentScope.findByRole("tab", { name: /事件记录|Events/i }));
  await waitFor(() => {
    expect(
      Array.from(MockEventSource.instances).some(
        (instance) => instance.readyState === MockEventSource.OPEN,
      ),
    ).toBe(true);
  });
  await new Promise((resolve) => setTimeout(resolve, 50));
  const descriptor = {
    topic: "prompt-cache.conversation-operations.window",
    params: { promptCacheKey: CONVERSATION_SHORT_KEY },
  };
  const items = operationEventsByPromptCacheKey.get(CONVERSATION_SHORT_KEY) ?? [];
  MockEventSource.emitMessage({
    type: "snapshot",
    topic: descriptor,
    topicKey: JSON.stringify(descriptor),
    schemaEpoch: "prompt-cache.conversation-operations.window/v1",
    cursor: 1,
    payload: {
      items,
      total: items.length,
      page: 1,
      pageSize: 20,
      routingModelFacets: ["gpt-5.4"],
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 50));
  MockEventSource.emitMessage({
    type: "live",
    topic: descriptor,
    topicKey: JSON.stringify(descriptor),
    schemaEpoch: "prompt-cache.conversation-operations.window/v1",
    cursor: 2,
    payload: {
      items,
      total: items.length,
      page: 1,
      pageSize: 20,
      routingModelFacets: ["gpt-5.4"],
    },
  });

  return documentScope;
}

async function assertDrawerOperationsStory(documentScope: ReturnType<typeof within>) {
  await expect(
    await documentScope.findByText(
      /查看当前对话的路由、正向代理与请求改写事件。|Review routing, forward-proxy, and request-rewrite events for this conversation\./i,
    ),
  ).toBeInTheDocument();
  await expect(documentScope.getAllByText(/路由相关|Routing/i).length).toBeGreaterThan(0);
  await expect(documentScope.getByText(/会话亲和性已重置|Affinity reset/i)).toBeInTheDocument();
  await expect(
    documentScope.getByText(/Sticky 目标已清空|Sticky target cleared/i),
  ).toBeInTheDocument();
  await expect(
    documentScope.getByText(/Sticky 目标已切换|Sticky target changed/i),
  ).toBeInTheDocument();
  await expect(documentScope.getAllByText(/系统自动|System auto/i).length).toBeGreaterThanOrEqual(
    2,
  );
  await expect(documentScope.getByText(/invokeId: invoke-short-32/i)).toBeInTheDocument();
  await expect(
    documentScope.getByText(
      /Sticky 目标：无 Sticky 目标 -> mia\.7rmmq@support\.example|Sticky target: No sticky target -> mia\.7rmmq@support\.example/i,
    ),
  ).toBeInTheDocument();
  await expect(
    documentScope.getByText(
      /mia\.7rmmq@support\.example 是唯一合格候选|mia\.7rmmq@support\.example was the only eligible candidate/i,
    ),
  ).toBeInTheDocument();
  await expect(
    documentScope.getByRole("link", { name: /查看选路决策|View routing decision/i }),
  ).toHaveAttribute("href", "/records?attemptId=SUCCESS32&invokeId=invoke-short-32");
  await expect(
    documentScope.getByRole("link", { name: /起因尝试：FAILED31|Cause attempt: FAILED31/i }),
  ).toHaveAttribute("href", "/records?attemptId=FAILED31");
  await expect(
    documentScope.getByRole("link", {
      name: /查看对应调用记录：invoke-short-32|View corresponding invocation: invoke-short-32/i,
    }),
  ).toHaveAttribute("href", "/records?attemptId=SUCCESS32&invokeId=invoke-short-32");
  await expect(documentScope.getByText("invoke-short-32")).toBeInTheDocument();
  await expect(documentScope.getAllByText(/Sticky 目标已切换|Sticky target changed/i).length).toBe(
    1,
  );
  await expect(
    documentScope.getByText(
      /绑定目标：账号 growth\.6vv4@relay\.example -> 无手工绑定|Binding: Account growth\.6vv4@relay\.example -> No manual binding/i,
    ),
  ).toBeInTheDocument();
  await expect(documentScope.queryByText(/策略更新|Policy updated/i)).not.toBeInTheDocument();
  await userEvent.click(documentScope.getByRole("button", { name: /路由相关|Routing/i }));
  const modelFilter = await documentScope.findByRole("combobox", {
    name: /路由模型|Routing model/i,
  });
  await expect(modelFilter).toHaveTextContent(/不限模型|Any model/i);
  await userEvent.click(modelFilter);
  const modelOptions = await documentScope.findByRole("listbox");
  await expect(modelOptions).toHaveTextContent(/全部模型范围|All-model scope/i);
  await userEvent.click(await documentScope.findByRole("option", { name: "gpt-5.4" }));
  await expect(modelFilter).toHaveTextContent("gpt-5.4");
  await expect(documentScope.getByText(/Sticky 目标已切换|Sticky target changed/i)).toBeVisible();
  await expect(
    documentScope.getAllByText(/(?:模型|Model)[:：]?\s*gpt-5\.4/i).length,
  ).toBeGreaterThan(0);
}

export async function playDrawerOperationsStory(canvasElement: HTMLElement) {
  const documentScope = await prepareDrawerOperationsStory(canvasElement);
  await assertDrawerOperationsStory(documentScope);
}

async function openDrawerRouting(canvasElement: HTMLElement) {
  bindingByPromptCacheKey.set(
    CONVERSATION_ROUTING_KEY,
    buildBindingResponse({
      promptCacheKey: CONVERSATION_ROUTING_KEY,
      bindingKind: "upstreamAccount",
      upstreamAccountId: 21,
      upstreamAccountName: "growth.6vv4@relay.example",
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 21,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "CIII",
      stickyRoutes: [
        {
          modelKey: null,
          upstreamAccountId: 21,
          upstreamAccountName: "growth.6vv4@relay.example",
          createdAt: "2026-05-13T23:40:00.000Z",
          updatedAt: "2026-05-13T23:42:00.000Z",
          lastSeenAt: "2026-05-13T23:47:36.000Z",
        },
        {
          modelKey: "gpt-5.4",
          upstreamAccountId: 22,
          upstreamAccountName: "mia.7rmmq@support.example",
          createdAt: "2026-05-13T23:43:00.000Z",
          updatedAt: "2026-05-13T23:47:36.000Z",
          lastSeenAt: "2026-05-13T23:47:39.000Z",
        },
      ],
    }),
  );
  const documentScope = within(canvasElement.ownerDocument.body);
  await userEvent.click(
    documentScope.getAllByRole("button", { name: /打开全部调用记录|open full call history/i })[0],
  );
  await userEvent.click(await documentScope.findByRole("tab", { name: /路由|Routing/i }));
  await waitFor(() => {
    expect(
      Array.from(MockEventSource.instances).some(
        (instance) => instance.readyState === MockEventSource.OPEN,
      ),
    ).toBe(true);
  });
  const bindingDescriptor = {
    topic: "prompt-cache.conversation-binding.current",
    params: { promptCacheKey: CONVERSATION_ROUTING_KEY },
  };
  MockEventSource.emitMessage({
    type: "snapshot",
    topic: bindingDescriptor,
    topicKey: JSON.stringify(bindingDescriptor),
    schemaEpoch: "prompt-cache.conversation-binding.current/v1",
    cursor: 1,
    payload: bindingByPromptCacheKey.get(CONVERSATION_ROUTING_KEY),
  });
  await expect(await documentScope.findByText(/当前路由|Current routing/i)).toBeInTheDocument();
  await expect(documentScope.getAllByText(/全部模型|All models/i).length).toBeGreaterThan(0);
  await expect(await documentScope.findAllByText(/gpt-5\.4/)).toHaveLength(2);
  await expect(documentScope.getAllByText("mia.7rmmq@support.example").length).toBeGreaterThan(0);

  return documentScope;
}

async function openAffinityResetConfirmation(canvasElement: HTMLElement) {
  const documentScope = await openDrawerRouting(canvasElement);
  await userEvent.click(
    documentScope.getByRole("button", { name: /清空绑定并重选|Clear binding and reselect/i }),
  );
  const resetConfirmation = await documentScope.findByRole("alertdialog", {
    name: /清空绑定并重选|Clear binding and reselect/i,
  });

  await expect(resetConfirmation).toBeInTheDocument();
  await expect(
    within(resetConfirmation).getByTestId("prompt-cache-affinity-reset-dialog-header"),
  ).toHaveClass("px-5", "pt-5", "pb-4");
  await expect(
    within(resetConfirmation).getByTestId("prompt-cache-affinity-reset-dialog-footer"),
  ).toHaveClass("border-t", "px-5", "pt-4");

  return { documentScope, resetConfirmation };
}

export type { Story };
export {
  largeHistoryStats,
  liveSyncSettledStats,
  meta,
  openAffinityResetConfirmation,
  openDrawerRouting,
  routingStoryStats,
  sharedScaleStats,
  shortSameDayStats,
};
