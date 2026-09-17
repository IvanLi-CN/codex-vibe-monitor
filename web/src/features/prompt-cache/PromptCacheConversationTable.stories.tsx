import { expect, fireEvent, fn, userEvent, waitFor, within } from "storybook/test";
import {
  bindingByPromptCacheKey,
  buildBindingResponse,
  CONVERSATION_LARGE_HISTORY_KEY,
  CONVERSATION_ONE_KEY,
  CONVERSATION_ROUTING_KEY,
  CONVERSATION_SHORT_KEY,
  largeHistory,
  largeHistoryStats,
  liveSyncSettledStats,
  MockEventSource,
  openAffinityResetConfirmation,
  openDrawerRouting,
  playDrawerBindingAndTimeoutsStory,
  playDrawerOperationsStory,
  queuedLargeHistoryRecord,
  routingStoryStats,
  type Story,
  sharedScaleStats,
  shortSameDayStats,
  stats,
  meta as storyMeta,
} from "./PromptCacheConversationTable.stories.support";

const meta = { ...storyMeta };
export default meta;

export const Populated: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
};

export const SingleExpanded: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
    expandedPromptCacheKeys: [stats.conversations[0]?.promptCacheKey ?? ""],
  },
};

export const ExpandAll: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
    expandedPromptCacheKeys: stats.conversations.map((conversation) => conversation.promptCacheKey),
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

export const LiveSyncSettled: Story = {
  args: {
    stats: liveSyncSettledStats,
    isLoading: false,
    error: null,
    expandedPromptCacheKeys: [CONVERSATION_ONE_KEY],
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable post-sync state after the Prompt Cache row consumes live `records` SSE updates and converges onto the final persisted invocation.",
      },
    },
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

export const DrawerOpen: Story = {
  args: {
    stats,
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await expect(
      await documentScope.findByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(/已加载 6 \/ 6 条保留调用记录|Loaded 6 \/ 6 retained record\(s\)/i),
    ).toBeInTheDocument();
    await expect(documentScope.getAllByTestId("invocation-table-scroll").length).toBeGreaterThan(0);
  },
};

export const ShortSameDayDrawerOpen: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Conversation history whose retained calls all occur within a short same-day window; the drawer chart should use the first and latest retained call timestamps instead of expanding to the full day.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await expect(
      await documentScope.findByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    const chart = await documentScope.findByTestId("conversation-activity-chart");
    await expect(chart).toHaveAttribute("data-chart-range-start", "2026-05-13T23:26:12.000Z");
    await expect(chart).toHaveAttribute("data-chart-range-end", "2026-05-13T23:40:47.000Z");
    await waitFor(() => {
      const successBars = Array.from(
        chart.querySelectorAll<SVGGraphicsElement>('path[fill="#22c55e"], rect[fill="#22c55e"]'),
      )
        .map((element) => element.getBBox())
        .filter((box) => box.width > 0 && box.height > 0);
      const failureBars = Array.from(
        chart.querySelectorAll<SVGGraphicsElement>('path[fill="#f87171"], rect[fill="#f87171"]'),
      )
        .map((element) => element.getBBox())
        .filter((box) => box.width > 0 && box.height > 0);
      expect(successBars.length).toBeGreaterThan(0);
      expect(failureBars.length).toBeGreaterThan(0);

      const alignedMiddleBucket = failureBars.some((failureBox) => {
        const failureCenter = failureBox.x + failureBox.width / 2;
        if (failureCenter < 200) return false;
        return successBars.some((successBox) => {
          const successCenter = successBox.x + successBox.width / 2;
          return Math.abs(successCenter - failureCenter) < 1;
        });
      });
      expect(alignedMiddleBucket).toBe(true);
    });
  },
};

export const DrawerBindingControls: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "History drawer with prompt-cache conversation route binding controls visible and preloaded with an upstream-account binding.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(await documentScope.findByRole("tab", { name: /路由|Routing/i }));
    await expect(await documentScope.findByText(/路由绑定|Route binding/i)).toBeInTheDocument();
    await expect(
      documentScope.getByText(
        /当前：账号 growth\.6vv4@relay\.example|Current: account growth\.6vv4@relay\.example/i,
      ),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(
        /加密会话 owner：growth\.6vv4@relay\.example · CIII|Encrypted session owner: growth\.6vv4@relay\.example · CIII/i,
      ),
    ).toBeInTheDocument();
    const bindingKindSelect = documentScope.getByRole("combobox", {
      name: /绑定类型|Binding type/i,
    });
    await expect(bindingKindSelect).toHaveTextContent(/上游账号|Account/i);

    await userEvent.click(bindingKindSelect);
    const bindingOptions = await documentScope.findByRole("listbox");
    await expect(bindingOptions).toHaveTextContent(/清空|Clear/i);
    await expect(bindingOptions).toHaveTextContent(/分组|Group/i);
    await expect(bindingOptions).toHaveTextContent(/上游账号|Account/i);
  },
};

export const DrawerRouting: Story = {
  tags: ["test"],
  args: {
    stats: routingStoryStats,
    isLoading: false,
    error: null,
    onOpenUpstreamAccount: fn(),
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Conversation routing tab with an all-model fallback, independent normalized model buckets, and the destructive affinity-reset confirmation.",
      },
    },
  },
  play: async ({ canvasElement, args }) => {
    const { documentScope, resetConfirmation } = await openAffinityResetConfirmation(canvasElement);
    await userEvent.click(within(resetConfirmation).getByRole("button", { name: /取消|Cancel/i }));
    await waitFor(() => {
      expect(documentScope.queryByRole("alertdialog")).toBeNull();
    });
    await expect(bindingByPromptCacheKey.get(CONVERSATION_ROUTING_KEY)?.stickyRoutes).toHaveLength(
      2,
    );

    await userEvent.click(
      documentScope.getByRole("button", {
        name: /查看 mia\.7rmmq@support\.example 的账号详情|View details for mia\.7rmmq@support\.example/i,
      }),
    );
    await expect(args.onOpenUpstreamAccount).toHaveBeenCalledTimes(1);
    await expect(args.onOpenUpstreamAccount).toHaveBeenCalledWith(22, "mia.7rmmq@support.example");
  },
};

export const DrawerRoutingResetConfirm: Story = {
  ...DrawerRouting,
  tags: ["test"],
  parameters: {
    docs: {
      description: {
        story:
          "Affinity reset confirmation with a dedicated padded content group and safe-area action footer.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    await openAffinityResetConfirmation(canvasElement);
  },
};

export const DrawerRoutingResetConfirmMobile: Story = {
  ...DrawerRoutingResetConfirm,
  tags: ["test"],
  globals: {
    themeMode: "dark",
    viewport: { value: "mobile393", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "The affinity reset confirmation at the stable 393 x 852 mobile viewport, including sheet-edge and safe-area spacing.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    await openAffinityResetConfirmation(canvasElement);
  },
};

export const DrawerRoutingMobile: Story = {
  ...DrawerRouting,
  globals: {
    themeMode: "dark",
    viewport: { value: "mobile393", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "The same current-routing and reset-confirmation state at the stable 393 x 852 mobile viewport.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = await openDrawerRouting(canvasElement);
    const drawerShell =
      canvasElement.ownerDocument.body.querySelector<HTMLElement>(".drawer-shell");
    const currentRouting = await documentScope.findByTestId("prompt-cache-current-routing");
    const mobileRouting = await documentScope.findByTestId("prompt-cache-current-routing-mobile");

    await expect(drawerShell).not.toBeNull();
    await expect(mobileRouting).toBeVisible();
    await expect(drawerShell?.scrollWidth).toBeLessThanOrEqual(drawerShell?.clientWidth ?? 0);
    await expect(currentRouting.scrollWidth).toBeLessThanOrEqual(currentRouting.clientWidth);
    await expect(mobileRouting.scrollWidth).toBeLessThanOrEqual(mobileRouting.clientWidth);
  },
};

export const DrawerEncryptedOwnerDangerConfirm: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Encrypted-session owner warning flow that uses the project dialog before changing a manually bound route.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    const originalConfirm = window.confirm;
    window.confirm = (message?: string) => {
      throw new Error(`Native confirm should not be used: ${String(message ?? "")}`);
    };

    try {
      await userEvent.click(historyButton);
      await userEvent.click(await documentScope.findByRole("tab", { name: /路由|Routing/i }));
      await expect(
        await documentScope.findByText(
          /加密会话 owner：growth\.6vv4@relay\.example · CIII|Encrypted session owner: growth\.6vv4@relay\.example · CIII/i,
        ),
      ).toBeInTheDocument();

      const bindingKindSelect = documentScope.getByRole("combobox", {
        name: /绑定类型|Binding type/i,
      });
      await userEvent.click(bindingKindSelect);
      await userEvent.click(await documentScope.findByRole("option", { name: /分组|Group/i }));

      await userEvent.click(documentScope.getByRole("button", { name: /保存|Save/i }));

      const confirmDialog = await documentScope.findByRole("alertdialog", {
        name: /要更改加密会话的路由绑定吗|change encrypted-session route binding/i,
      });
      await expect(confirmDialog).toHaveTextContent(/growth\.6vv4@relay\.example · CIII/i);
      await expect(confirmDialog).toHaveTextContent(/invalid_encrypted_content/i);
      await userEvent.click(within(confirmDialog).getByRole("button", { name: /取消|Cancel/i }));
      await waitFor(() => {
        expect(documentScope.queryByRole("alertdialog")).toBeNull();
      });
      await expect(
        documentScope.getByText(
          /当前：账号 growth\.6vv4@relay\.example|Current: account growth\.6vv4@relay\.example/i,
        ),
      ).toBeInTheDocument();
    } finally {
      window.confirm = originalConfirm;
    }
  },
};

export const DrawerEncryptedOwnerDangerDialogOpen: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable visual state for the encrypted-session owner warning dialog inside the conversation route-binding drawer.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    const originalConfirm = window.confirm;
    window.confirm = (message?: string) => {
      throw new Error(`Native confirm should not be used: ${String(message ?? "")}`);
    };

    try {
      await userEvent.click(historyButton);
      await userEvent.click(await documentScope.findByRole("tab", { name: /路由|Routing/i }));
      const bindingKindSelect = documentScope.getByRole("combobox", {
        name: /绑定类型|Binding type/i,
      });
      await userEvent.click(bindingKindSelect);
      await userEvent.click(await documentScope.findByRole("option", { name: /分组|Group/i }));

      await userEvent.click(documentScope.getByRole("button", { name: /保存|Save/i }));

      const confirmDialog = await documentScope.findByRole("alertdialog", {
        name: /要更改加密会话的路由绑定吗|change encrypted-session route binding/i,
      });
      await expect(confirmDialog).toHaveTextContent(/growth\.6vv4@relay\.example · CIII/i);
      await expect(confirmDialog).toHaveTextContent(/invalid_encrypted_content/i);
    } finally {
      window.confirm = originalConfirm;
    }
  },
};

export const DrawerOwnerLockWithoutManualBinding: Story = {
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Expanded Prompt Cache drawer that shows the encrypted owner hint after manual binding has been cleared.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    bindingByPromptCacheKey.set(
      CONVERSATION_SHORT_KEY,
      buildBindingResponse({
        promptCacheKey: CONVERSATION_SHORT_KEY,
        bindingKind: "none",
        hasEncryptedSessionOwner: true,
        encryptedOwnerAccountId: 21,
        encryptedOwnerAccountName: "growth.6vv4@relay.example",
        encryptedOwnerGroupName: "CIII",
        timeouts: {
          responsesFirstByteTimeoutSecs: 35,
          compactFirstByteTimeoutSecs: 180,
          responsesStreamTimeoutSecs: 210,
          compactStreamTimeoutSecs: 300,
        },
        timeoutFieldSources: {
          responsesFirstByteTimeoutSecs: "conversation",
          compactFirstByteTimeoutSecs: "account",
          responsesStreamTimeoutSecs: "conversation",
          compactStreamTimeoutSecs: "root",
        },
      }),
    );
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(await documentScope.findByRole("tab", { name: /路由|Routing/i }));
    await expect(await documentScope.findByText(/路由绑定|Route binding/i)).toBeInTheDocument();
    await expect(
      documentScope.getByText(/当前：无手工绑定|Current: no manual binding/i),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByText(
        /清空手工绑定不会清除加密会话 owner 锁|Clearing the manual binding does not remove the encrypted session owner lock/i,
      ),
    ).toBeInTheDocument();
  },
};

export const DrawerBindingAndTimeouts: Story = {
  tags: ["test"],
  parameters: {
    a11y: {
      test: "off",
    },
    docs: {
      description: {
        story:
          "Prompt Cache drawer showing the widened conversation detail panel plus the account-style routing form for conversation overrides, with mixed conversation/account/root sources across policy rows and timeouts.",
      },
    },
  },
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  play: ({ canvasElement }) => playDrawerBindingAndTimeoutsStory(canvasElement),
};

export const DrawerOperations: Story = {
  tags: ["test"],
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Prompt Cache drawer showing the affinity reset sequence: clear the existing sticky target, suppress stale in-flight sticky resurrection, then record exactly one fresh systemAuto sticky reassignment event with invokeId.",
      },
    },
  },
  play: ({ canvasElement }) => playDrawerOperationsStory(canvasElement),
};

export const LargeHistoryVirtualizedDrawer: Story = {
  args: {
    stats: largeHistoryStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Large retained conversation history with 15,000 total rows; the drawer loads 50 rows first and relies on virtualized visible rows while preserving route-binding controls.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(await documentScope.findByRole("tab", { name: /调用|Calls/i }));
    await expect(
      await documentScope.findByText(
        /已加载 50 \/ 15,?000 条保留调用记录|Loaded 50 \/ 15,?000 retained record\(s\)/i,
      ),
    ).toBeInTheDocument();
    expect(canvasElement.ownerDocument.body.querySelectorAll("tbody tr").length).toBeLessThan(90);

    const drawerBody = canvasElement.ownerDocument.body.querySelector(".drawer-body");
    expect(drawerBody).toBeTruthy();
    if (drawerBody instanceof HTMLElement) {
      drawerBody.scrollTop = drawerBody.scrollHeight;
      fireEvent.scroll(drawerBody);
    }

    await expect(
      await documentScope.findByText(
        /已加载 100 \/ 15,?000 条保留调用记录|Loaded 100 \/ 15,?000 retained record\(s\)/i,
      ),
    ).toBeInTheDocument();
  },
};

export const DrawerQueuedRealtimeCalls: Story = {
  tags: ["test"],
  args: {
    stats: largeHistoryStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "dark",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "Conversation calls preserve the reader's position away from the top and expose a deterministic new-record prompt instead of inserting the live row immediately.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const historyButton = documentScope.getAllByRole("button", {
      name: /打开全部调用记录|open full call history/i,
    })[0];

    await userEvent.click(historyButton);
    await userEvent.click(await documentScope.findByRole("tab", { name: /调用|Calls/i }));
    await waitFor(() => {
      expect(
        Array.from(MockEventSource.instances).some(
          (instance) => instance.readyState === MockEventSource.OPEN,
        ),
      ).toBe(true);
    });

    const descriptor = {
      topic: "invocation-history.window",
      params: { promptCacheKey: CONVERSATION_LARGE_HISTORY_KEY },
    };
    const initialRecords = largeHistory.slice(0, 50);
    MockEventSource.emitMessage({
      type: "snapshot",
      topic: descriptor,
      topicKey: JSON.stringify(descriptor),
      schemaEpoch: "invocation-history.window/v1",
      cursor: 1,
      payload: {
        snapshotId: 8401,
        total: largeHistory.length,
        page: 1,
        pageSize: 50,
        records: initialRecords,
      },
    });
    await expect(
      await documentScope.findByText(
        /已加载 50 \/ 15,?000 条保留调用记录|Loaded 50 \/ 15,?000 retained record\(s\)/i,
      ),
    ).toBeInTheDocument();
    const drawerBody = canvasElement.ownerDocument.body.querySelector(".drawer-body");
    if (!(drawerBody instanceof HTMLElement)) {
      throw new Error("missing drawer body");
    }
    drawerBody.scrollTop = 144;
    fireEvent.scroll(drawerBody);
    MockEventSource.emitMessage({
      type: "live",
      topic: descriptor,
      topicKey: JSON.stringify(descriptor),
      schemaEpoch: "invocation-history.window/v1",
      cursor: 2,
      payload: {
        snapshotId: 8402,
        total: largeHistory.length + 1,
        page: 1,
        pageSize: 50,
        records: [queuedLargeHistoryRecord, ...initialRecords],
      },
    });

    await expect(
      await documentScope.findByRole("button", { name: /查看 1 条新记录|Show 1 new record/i }),
    ).toBeInTheDocument();
  },
};

export const DrawerBindingRemoteConflict: Story = {
  tags: ["test"],
  args: {
    stats: shortSameDayStats,
    isLoading: false,
    error: null,
  },
  globals: {
    themeMode: "light",
    viewport: { value: "desktop1280", isRotated: false },
  },
  parameters: {
    docs: {
      description: {
        story:
          "An external binding topic update does not overwrite a local draft; it presents explicit adopt-latest and last-write-wins choices.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const initialBinding = buildBindingResponse({
      promptCacheKey: CONVERSATION_SHORT_KEY,
      bindingKind: "upstreamAccount",
      upstreamAccountId: 21,
      upstreamAccountName: "growth.6vv4@relay.example",
      hasEncryptedSessionOwner: true,
      encryptedOwnerAccountId: 21,
      encryptedOwnerAccountName: "growth.6vv4@relay.example",
      encryptedOwnerGroupName: "CIII",
      updatedAt: "2026-05-13T23:42:00.000Z",
    });
    bindingByPromptCacheKey.set(CONVERSATION_SHORT_KEY, initialBinding);
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
      payload: initialBinding,
    });
    const kindSelect = await documentScope.findByRole("combobox", {
      name: /绑定类型|Binding type/i,
    });
    await userEvent.click(kindSelect);
    await userEvent.click(await documentScope.findByRole("option", { name: /分组|Group/i }));
    await waitFor(() => {
      expect(
        Array.from(MockEventSource.instances).some(
          (instance) => instance.readyState === MockEventSource.OPEN,
        ),
      ).toBe(true);
    });

    MockEventSource.emitMessage({
      type: "live",
      topic: descriptor,
      topicKey: JSON.stringify(descriptor),
      schemaEpoch: "prompt-cache.conversation-binding.current/v1",
      cursor: 3,
      payload: buildBindingResponse({
        promptCacheKey: CONVERSATION_SHORT_KEY,
        bindingKind: "none",
        updatedAt: "2026-05-13T23:43:00.000Z",
      }),
    });

    await expect(
      await documentScope.findByText(
        /编辑期间，此对话的路由绑定已在其他位置更新。|changed elsewhere while you were editing/i,
      ),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByRole("button", { name: /采用最新配置|Use latest/i }),
    ).toBeInTheDocument();
    await expect(
      documentScope.getByRole("button", { name: /仍然保存|Save mine/i }),
    ).toBeInTheDocument();
  },
};
