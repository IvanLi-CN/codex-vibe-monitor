import { expect, userEvent, waitFor, within } from "storybook/test";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";
import { UpstreamAccountTab } from "./DashboardWorkingConversationsSection.stories";
import {
  assertQuickPolicyTonePalette,
  BulkSelectionStorySurface,
  buildCards,
  bulkSelectionStoryArgs,
  bulkSelectionStoryRecentTargets,
  createConversation,
  createdAtDescendingOrderCards,
  createdAtDescendingOrderKeys,
  createPreview,
  createRelativeStoryIso,
  createResponse,
  createUpstreamAccountActivityStoryResponse,
  createUpstreamAccountAdaptiveMetricsStoryResponse,
  DashboardAccountWindowEvidenceSurface,
  DrawerPreviewStory,
  enableConversationSelectionMode,
  ForcedWorkspaceViewStory,
  failedClickableResponse,
  fourCardParallelThreeSlotProofResponse,
  getStorySequenceIdForPromptCacheKey,
  HeadInsertAnchorStory,
  LONG_ERROR_SUMMARY,
  openBulkClearBindingDialog,
  readPerceptualChannel,
  requireFixture,
  type Story,
  selectConversationForBulkActions,
  meta as storyMeta,
  upstreamAccountSortOrderingResponse,
  virtualizedLargeDatasetCards,
  wideDesktopResponse,
} from "./DashboardWorkingConversationsSection.stories.support";

const meta = { ...storyMeta };
export default meta;

export const UpstreamAccountHeaderActions: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-routing-badges", [
          createPreview({
            id: 9861,
            invokeId: "story-working-routing-badges",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const attentionBadges = await canvas.findByTestId(
      "dashboard-upstream-account-attention-badges",
    );
    const [firstAttentionBadge] = await canvas.findAllByTestId(
      "dashboard-upstream-account-attention-badge",
    );
    expect(attentionBadges.querySelector("button")).toBe(firstAttentionBadge);
    await userEvent.click(firstAttentionBadge);
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "account:42:healthEvents",
    );
    await expect(within(document.body).getByTestId("story-account-drawer-tab")).toHaveTextContent(
      "Tab healthEvents",
    );
    await userEvent.click(
      within(document.body).getByRole("button", {
        name: "Close account drawer",
      }),
    );
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent("none");
    await expect(canvas.queryByTestId("dashboard-upstream-account-routing-settings")).toBeNull();

    const policyBadges = await canvas.findAllByTestId("dashboard-upstream-account-policy-badge");
    await userEvent.click(requireFixture(policyBadges[0]));
    await expect(requireFixture(policyBadges[1])).toHaveTextContent("强制Fast");
    await expect(requireFixture(policyBadges[1])).toHaveAttribute(
      "aria-label",
      expect.stringContaining("Fast 改写策略：强制Fast"),
    );
    await userEvent.click(requireFixture(policyBadges[1]));
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent("none");
    await waitFor(
      () => {
        const patchLog = (
          window as typeof window & {
            __dashboardStoryPolicyPatchLog?: string[];
          }
        ).__dashboardStoryPolicyPatchLog;
        expect(patchLog?.[0]).toContain('"priorityTier":"normal"');
        expect(patchLog?.[0]).toContain('"fastModeRewriteMode":"force_remove"');
      },
      { timeout: 1600 },
    );
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dashboard upstream-account card header actions: attention badges open health events, the gear opens routing, and quick policy chips including Fast rewrite labels save account-level overrides with a debounced PATCH.",
      },
    },
  },
};

export const UpstreamAccountQuickPolicyTonePalette: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DashboardAccountWindowEvidenceSurface>
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-policy-tones", [
            createPreview({
              id: 9871,
              invokeId: "story-working-policy-tones",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(4, {
          allowCutOut: false,
          allowCutIn: true,
          priorityTier: "fallback",
          fastModeRewriteMode: "force_add",
        })}
      />
    </DashboardAccountWindowEvidenceSurface>
  ),
  play: async ({ canvasElement }) => assertQuickPolicyTonePalette(canvasElement),
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dashboard upstream-account quick policy chips shown as a semantic tone palette: fallback uses success, force Fast uses primary, active cut-out block uses warning, and inactive cut-in remains neutral.",
      },
    },
  },
};

export const UpstreamAccountQuickPolicyTonePaletteDark: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DashboardAccountWindowEvidenceSurface>
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-policy-tones-dark", [
            createPreview({
              id: 9872,
              invokeId: "story-working-policy-tones-dark",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(4, {
          allowCutOut: false,
          allowCutIn: true,
          priorityTier: "fallback",
          fastModeRewriteMode: "force_add",
        })}
        theme="vibe-dark"
      />
    </DashboardAccountWindowEvidenceSurface>
  ),
  play: async ({ canvasElement }) => assertQuickPolicyTonePalette(canvasElement),
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dark theme checkpoint for the dashboard upstream-account quick policy tone palette.",
      },
    },
  },
};

export const UpstreamAccountQuickPolicyTonePaletteMobile: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DashboardAccountWindowEvidenceSurface>
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-policy-tones-mobile", [
            createPreview({
              id: 9873,
              invokeId: "story-working-policy-tones-mobile",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(4, {
          allowCutOut: false,
          allowCutIn: true,
          priorityTier: "fallback",
          fastModeRewriteMode: "force_add",
        })}
      />
    </DashboardAccountWindowEvidenceSurface>
  ),
  play: async ({ canvasElement }) => assertQuickPolicyTonePalette(canvasElement),
  parameters: {
    viewport: { defaultViewport: "mobile393" },
    docs: {
      description: {
        story:
          "Mobile responsive checkpoint for the dashboard upstream-account quick policy chips.",
      },
    },
  },
};

export const UpstreamAccountMetricTooltips: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account-tooltips", [
          createPreview({
            id: 9831,
            invokeId: "story-working-tooltips",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const triggers = await canvas.findAllByTestId("dashboard-upstream-account-metric-card");
    await expect(triggers).toHaveLength(4);

    const tpmInlineMetric = canvas.getByLabelText("TPM 37,280");
    await userEvent.click(tpmInlineMetric);
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain("TPM 37,280");
    });
    await userEvent.click(tpmInlineMetric);

    const assertMetricTooltip = async (metric: string, expectedTexts: string[]) => {
      const trigger = canvasElement.querySelector(
        `[data-testid="dashboard-upstream-account-metric-card"][data-metric="${metric}"]`,
      );
      if (!(trigger instanceof HTMLElement)) {
        throw new Error(`missing ${metric} metric trigger`);
      }
      await userEvent.click(trigger);
      await waitFor(() => {
        const tooltipText = document.body.textContent ?? "";
        for (const text of expectedTexts) {
          expect(tooltipText).toContain(text);
        }
      });
      await userEvent.click(trigger);
      await userEvent.unhover(trigger);
    };

    await assertMetricTooltip("latency", ["TTFT", "4.38 s", "响应时间", "阶段首字节"]);
    await assertMetricTooltip("requests", ["请求数", "成功率", "75%", "非成功率"]);
    await assertMetricTooltip("cost", [
      "用量明细",
      "3.85",
      "缓存写入",
      "缓存读取",
      "总计",
      "gpt-5.5",
    ]);
    await assertMetricTooltip("token", [
      "用量明细",
      "缓存写入",
      "缓存读取",
      "输出",
      "总计",
      "gpt-5.5",
    ]);

    const finalTrigger = canvasElement.querySelector(
      '[data-testid="dashboard-upstream-account-metric-card"][data-metric="cost"]',
    );
    if (finalTrigger instanceof HTMLElement) await userEvent.click(finalTrigger);
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Stable interaction coverage for the four upstream-account metric cards. Each whole metric card opens a structured tooltip with explicit field labels, values, and related computed data while the card surface stays compact.",
      },
    },
  },
};

export const UpstreamAccountAdaptiveMetricHardThreshold: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-hard-threshold", [
            createPreview({
              id: 9831,
              invokeId: "story-working-hard-threshold",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountAdaptiveMetricsStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  decorators: [
    (Story) => (
      <div
        data-testid="dashboard-upstream-account-adaptive-hard-frame"
        className="dashboard-upstream-account-adaptive-hard-story"
      >
        <style>{`
          .dashboard-upstream-account-adaptive-hard-story {
            display: inline-block;
            padding: 16px;
            background: oklch(var(--color-base-300));
            border: 1px solid rgba(100, 116, 139, 0.7);
          }
          .dashboard-upstream-account-adaptive-hard-story [data-testid="dashboard-working-conversations"] {
            display: inline-block;
            width: fit-content;
            background: transparent;
          }
          .dashboard-upstream-account-adaptive-hard-story
            [data-testid="dashboard-working-conversations"]
            > .surface-panel-body {
            width: fit-content;
            padding: 0 !important;
            background: transparent;
          }
          .dashboard-upstream-account-adaptive-hard-story
            [data-testid="dashboard-upstream-account-grid"] {
            width: 52rem;
            grid-template-columns: minmax(0, 1fr) !important;
          }
          .dashboard-upstream-account-adaptive-hard-story
            [data-testid="dashboard-working-conversations-controls"],
          .dashboard-upstream-account-adaptive-hard-story
            [data-testid="dashboard-upstream-account-header-row"],
          .dashboard-upstream-account-adaptive-hard-story
            [data-testid="dashboard-upstream-account-recent-section"],
          .dashboard-upstream-account-adaptive-hard-story
            div:has(> [data-testid="story-drawer-state"]) {
            display: none;
          }
        `}</style>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await waitFor(() => {
      expect(canvas.getByTestId("dashboard-upstream-account-requests-value")).toHaveTextContent(
        "10,376",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-requests-value")).toHaveAttribute(
        "data-compact",
        "false",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-cost-value")).toHaveTextContent(
        "$30.0M",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-cost-value")).toHaveAttribute(
        "title",
        "30,030,779.25",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-token-value")).toHaveTextContent(
        "30.0M",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-token-value")).toHaveAttribute(
        "title",
        "30,030,779",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-latency-value")).toHaveTextContent(
        "1.08 min",
      );
    });
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Wide desktop account-card state proving that values with two grouping separators compact immediately while a one-group request count remains complete. Exact values remain available through card titles and structured details.",
      },
    },
  },
};

export const UpstreamAccountAdaptiveMetricResponsive: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-responsive-metrics", [
            createPreview({
              id: 9833,
              invokeId: "story-working-responsive-metrics",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountAdaptiveMetricsStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  decorators: [
    (Story) => (
      <div
        data-testid="dashboard-upstream-account-adaptive-responsive-frame"
        className="dashboard-upstream-account-adaptive-responsive-story"
      >
        <style>{`
          .dashboard-upstream-account-adaptive-responsive-story {
            display: inline-block;
            padding: 16px;
            background: oklch(var(--color-base-300));
            border: 1px solid rgba(100, 116, 139, 0.7);
          }
          .dashboard-upstream-account-adaptive-responsive-story [data-testid="dashboard-working-conversations"] {
            display: inline-block;
            width: fit-content;
            background: transparent;
          }
          .dashboard-upstream-account-adaptive-responsive-story
            [data-testid="dashboard-working-conversations"]
            > .surface-panel-body {
            width: fit-content;
            padding: 0 !important;
            background: transparent;
          }
          .dashboard-upstream-account-adaptive-responsive-story
            [data-testid="dashboard-working-conversations-controls"],
          .dashboard-upstream-account-adaptive-responsive-story
            [data-testid="dashboard-upstream-account-header-row"],
          .dashboard-upstream-account-adaptive-responsive-story
            [data-testid="dashboard-upstream-account-recent-section"],
          .dashboard-upstream-account-adaptive-responsive-story
            div:has(> [data-testid="story-drawer-state"]) {
            display: none;
          }
        `}</style>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await waitFor(() => {
      expect(canvas.getByTestId("dashboard-upstream-account-requests-value")).toHaveTextContent(
        "10,376",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-cost-value")).toHaveTextContent(
        "$30.0M",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-token-value")).toHaveTextContent(
        "30.0M",
      );
      expect(canvas.getByTestId("dashboard-upstream-account-latency-value")).toHaveTextContent(
        "1.08 min",
      );
      expect(document.documentElement.scrollWidth).toBeLessThanOrEqual(
        document.documentElement.clientWidth,
      );
    });
  },
  parameters: {
    viewport: { defaultViewport: "mobile393" },
    docs: {
      description: {
        story:
          "Mobile 393 x 852 account-card metric surface showing the real single-account-column layout with two metric columns and no horizontal overflow.",
      },
    },
  },
};

export const UpstreamAccountAdaptiveMetricOverflow: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-adaptive-overflow", [
            createPreview({
              id: 9832,
              invokeId: "story-working-adaptive-overflow",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountAdaptiveMetricsStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  decorators: [
    (Story) => (
      <div
        data-testid="dashboard-upstream-account-adaptive-evidence-frame"
        className="dashboard-upstream-account-adaptive-overflow-story"
      >
        <style>{`
          .dashboard-upstream-account-adaptive-overflow-story {
            display: inline-block;
            padding: 19px 19px 12px;
            background: color-mix(
              in oklab,
              oklch(var(--color-base-100)) 74%,
              oklch(var(--color-info)) 26%
            );
          }
          .dashboard-upstream-account-adaptive-overflow-story [data-testid="dashboard-working-conversations"] {
            display: inline-block;
            width: fit-content;
          }
          .dashboard-upstream-account-adaptive-overflow-story
            [data-testid="dashboard-working-conversations"]
            > .surface-panel-body {
            width: fit-content;
          }
          .dashboard-upstream-account-adaptive-overflow-story [data-testid="dashboard-upstream-account-grid"] {
            max-width: 22rem;
            grid-template-columns: minmax(0, 1fr) !important;
          }
          .dashboard-upstream-account-adaptive-overflow-story
            [data-testid="dashboard-working-conversations-controls"] {
            display: none;
          }
          .dashboard-upstream-account-adaptive-overflow-story
            div:has(> [data-testid="story-drawer-state"]) {
            display: none;
          }
        `}</style>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await waitFor(() => {
      expect(canvas.getByText("当前活动账号 1 个")).toBeInTheDocument();

      const tpmValue = canvas.getByTestId("dashboard-upstream-account-inline-tpm-value");
      const spendRateValue = canvas.getByTestId(
        "dashboard-upstream-account-inline-spend-rate-value",
      );
      const costValue = canvas.getByTestId("dashboard-upstream-account-cost-value");
      const tokenValue = canvas.getByTestId("dashboard-upstream-account-token-value");
      const accountCard = canvas.getByTestId("dashboard-upstream-account-card");
      const recentBreakdown = canvas.getByTestId("dashboard-upstream-account-recent-breakdown");
      const phaseSegments = canvas.getAllByTestId("invocation-phase-segment");

      expect(accountCard).toHaveAttribute("data-header-layout", "stacked");
      expect(accountCard).toHaveAttribute("data-inline-metric-layout", "three-columns");
      expect(accountCard).toHaveAttribute("data-metric-columns", "2");
      expect(tpmValue).toHaveAttribute("data-compact", "true");
      expect(tpmValue.textContent ?? "").toMatch(/M|B|T/);
      expect(tpmValue).toHaveAttribute("title", "1,324,743");

      expect(spendRateValue).toHaveAttribute("data-compact", "false");
      expect(spendRateValue).toHaveTextContent("54");
      expect(spendRateValue).not.toHaveAttribute("title");

      expect(costValue).toHaveAttribute("data-compact", "true");
      expect(costValue).toHaveTextContent("$30.0M");
      expect(costValue).toHaveAttribute("title", "30,030,779.25");

      expect(tokenValue).toHaveAttribute("data-compact", "true");
      expect(tokenValue.textContent ?? "").toMatch(/M|B|T/);
      expect(tokenValue).toHaveAttribute("title", "30,030,779");

      expect(recentBreakdown.textContent ?? "").not.toContain("排队中");
      expect(recentBreakdown.textContent ?? "").not.toContain("请求中");
      expect(recentBreakdown.textContent ?? "").not.toContain("响应中");
      expect(recentBreakdown.textContent ?? "").not.toContain("失败");
      expect(recentBreakdown.textContent ?? "").not.toContain("成功");
      expect(phaseSegments[0]).toHaveAttribute("data-phase-label-visible", "false");
    });
  },
  parameters: {
    viewport: { defaultViewport: "mobile393" },
    docs: {
      description: {
        story:
          "Narrow owner-facing upstream-account card that forces the red-box metrics to reuse the adaptive compact-number system. Inline TPM/spend-rate and the hero cost/token values must collapse precision or magnitude while preserving the full value in tooltips and titles.",
      },
    },
  },
};

export const UpstreamAccountSplitHeaderTpmWidthBudget: Story = {
  args: UpstreamAccountTab.args,
  render: () => {
    const upstreamAccountActivity = createUpstreamAccountActivityStoryResponse();
    const account = upstreamAccountActivity.accounts[0];
    if (account) {
      account.tokensPerMinute = 2_027_266;
      account.spendRate = 0.85;
      account.inProgressInvocationCount = 6;
      account.uploadBytesPerSecond = 614.5 * 1024;
      account.downloadBytesPerSecond = 20.9 * 1024;
      account.modelPerformance = {
        available: true,
        total: {
          tokensPerMinute: 2_027_266,
          streamingResponseRate: 19.8,
          avgResponseMs: 18_420,
          avgFirstTokenMs: 4_380,
          wallClockUsageDurationMs: 42_000,
          cumulativeUsageDurationMs: 51_000,
          parallelism: 1.2,
        },
        models: [
          {
            model: "gpt-5.6",
            reasoningEffort: "medium",
            tokensPerMinute: 2_027_266,
            streamingResponseRate: 19.8,
            avgResponseMs: 18_420,
            avgFirstTokenMs: 4_380,
            wallClockUsageDurationMs: 42_000,
            cumulativeUsageDurationMs: 51_000,
            parallelism: 1.2,
          },
        ],
      };
    }

    return (
      <ForcedWorkspaceViewStory view="upstreamAccounts">
        <DrawerPreviewStory
          response={createResponse([
            createConversation("pck-story-upstream-account-split-tpm-budget", [
              createPreview({
                id: 9833,
                invokeId: "story-working-split-tpm-budget",
                occurredAt: "2026-04-04T10:05:00Z",
                status: "running",
                upstreamAccountId: 42,
                upstreamAccountName: "Pool Alpha",
              }),
            ]),
          ])}
          upstreamAccountActivity={upstreamAccountActivity}
        />
      </ForcedWorkspaceViewStory>
    );
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    await waitFor(() => {
      const accountCard = canvas.getByTestId("dashboard-upstream-account-card");
      const tpmValue = canvas.getByTestId("dashboard-upstream-account-inline-tpm-value");
      const spendRateValue = canvas.getByTestId(
        "dashboard-upstream-account-inline-spend-rate-value",
      );

      expect(accountCard).toHaveAttribute("data-header-layout", "split");
      expect(tpmValue).toHaveAttribute("data-compact", "true");
      expect(tpmValue.textContent ?? "").toMatch(/M|B|T/);
      expect(tpmValue).toHaveAttribute("title", "2,027,266");
      expect(spendRateValue).toHaveAttribute("data-compact", "false");
      expect(spendRateValue).toHaveTextContent("0.85");
      expect(canvas.getByLabelText("TPM 2,027,266 Pool Alpha · 模型性能")).toBeInTheDocument();
      expect(canvas.getByLabelText("消费速率 0.85 Pool Alpha · 模型性能")).toBeInTheDocument();
      expect(canvas.getByLabelText("进行中 6")).toBeInTheDocument();
    });
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Wide upstream-account header that keeps the split layout while forcing long TPM values through the dedicated ~6ch budget. Only TPM compacts; in-progress and spend-rate stay on their current display path while full TPM semantics remain available via title and aria-label.",
      },
    },
  },
};

export const ErrorSummaryTooltips: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(
      createResponse([
        createConversation("pck-story-error-summary-tooltips", [
          createPreview({
            id: 9941,
            invokeId: "story-error-summary-current",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "http_502",
            failureClass: "service_failure",
            failureKind: "upstream_http_5xx",
            errorMessage: LONG_ERROR_SUMMARY,
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
            tUpstreamTtfbMs: null,
            tUpstreamStreamMs: null,
            tTotalMs: 18_420,
          }),
          createPreview({
            id: 9940,
            invokeId: "story-error-summary-previous",
            occurredAt: "2026-04-04T10:03:00Z",
            status: "success",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ]),
    ),
    isLoading: false,
    error: null,
  },
  render: () => {
    const upstreamAccountActivity = createUpstreamAccountActivityStoryResponse();
    upstreamAccountActivity.accounts[0] = {
      ...upstreamAccountActivity.accounts[0],
      recentInvocations: upstreamAccountActivity.accounts[0].recentInvocations.map(
        (invocation, index) =>
          index === 0
            ? {
                ...invocation,
                status: "http_502",
                failureClass: "service_failure",
                failureKind: "upstream_http_5xx",
                errorMessage: LONG_ERROR_SUMMARY,
                tUpstreamTtfbMs: null,
                tUpstreamStreamMs: null,
                tTotalMs: 21_006,
              }
            : invocation,
      ),
    };

    return (
      <ForcedWorkspaceViewStory view="conversations">
        <DrawerPreviewStory
          response={createResponse([
            createConversation("pck-story-error-summary-tooltips", [
              createPreview({
                id: 9941,
                invokeId: "story-error-summary-current",
                occurredAt: "2026-04-04T10:05:00Z",
                status: "http_502",
                failureClass: "service_failure",
                failureKind: "upstream_http_5xx",
                errorMessage: LONG_ERROR_SUMMARY,
                upstreamAccountId: 42,
                upstreamAccountName: "Pool Alpha",
                tUpstreamTtfbMs: null,
                tUpstreamStreamMs: null,
                tTotalMs: 18_420,
              }),
              createPreview({
                id: 9940,
                invokeId: "story-error-summary-previous",
                occurredAt: "2026-04-04T10:03:00Z",
                status: "success",
                upstreamAccountId: 42,
                upstreamAccountName: "Pool Alpha",
              }),
            ]),
          ])}
          upstreamAccountActivity={upstreamAccountActivity}
        />
      </ForcedWorkspaceViewStory>
    );
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing failed current slot");
    }

    const slotErrorSummary = currentSlot.querySelector('[data-testid="invocation-error-summary"]');
    const slotErrorTrigger = slotErrorSummary?.parentElement;
    if (!(slotErrorSummary instanceof HTMLElement) || !(slotErrorTrigger instanceof HTMLElement)) {
      throw new Error("missing current slot error summary trigger");
    }

    await userEvent.hover(slotErrorTrigger);
    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll("[data-side]")).find((node) =>
        node.textContent?.includes(LONG_ERROR_SUMMARY),
      );
      expect(tooltip?.textContent).toContain(LONG_ERROR_SUMMARY);
      expect(tooltip?.getAttribute("data-side")).toBe("bottom");
    });
    await userEvent.unhover(slotErrorTrigger);

    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const recentRow = await canvas.findByTestId("dashboard-upstream-account-recent-row");
    const recentErrorSummary = recentRow.querySelector('[data-testid="invocation-error-summary"]');
    const recentErrorTrigger = recentErrorSummary?.parentElement;
    if (
      !(recentErrorSummary instanceof HTMLElement) ||
      !(recentErrorTrigger instanceof HTMLElement)
    ) {
      throw new Error("missing recent row error summary trigger");
    }

    const accountGrid = canvasElement.querySelector(
      '[data-testid="dashboard-upstream-account-grid"]',
    );
    const accountCard = recentRow.closest('[data-testid="dashboard-upstream-account-card"]');
    if (!(accountGrid instanceof HTMLElement) || !(accountCard instanceof HTMLElement)) {
      throw new Error("missing upstream account layout shrink chain");
    }

    expect(accountGrid.className).toContain("desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]");
    expect(accountGrid.className).toContain("items-start");
    expect(accountCard.className).toContain("min-w-0");
    expect(accountCard.className).not.toContain("h-full");
    expect(accountCard.className).not.toContain("desktop1660:min-h-[31.5rem]");
    expect(recentRow.className).toContain("min-w-0");
    expect(recentErrorTrigger.className).toContain("w-full");
    expect(recentErrorTrigger.className).toContain("overflow-hidden");

    await userEvent.hover(recentErrorTrigger);
    await waitFor(() => {
      const tooltip = Array.from(document.body.querySelectorAll("[data-side]")).find((node) =>
        node.textContent?.includes(LONG_ERROR_SUMMARY),
      );
      expect(tooltip?.textContent).toContain(LONG_ERROR_SUMMARY);
      expect(tooltip?.getAttribute("data-side")).toBe("bottom");
    });
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Long failed invocation summaries stay single-line and truncated inside both the current slot and upstream-account recent rows, while hover opens the shared tooltip below the trigger with the full upstream error payload.",
      },
    },
  },
};

export const UpstreamAccountRecentIdentityChipOpensConversation: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account", [
          createPreview({
            id: 9801,
            invokeId: "story-working-invoke",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const identityChip = canvas.getAllByTestId(
      "dashboard-upstream-account-recent-identity-chip",
    )[0];
    if (!(identityChip instanceof HTMLButtonElement)) {
      throw new Error("expected upstream identity chip button");
    }

    await userEvent.click(identityChip);
    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-drawer-state"]')?.textContent,
      ).toContain("conversation:pck-upstream-running");
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "conversation:pck-upstream-running",
    );

    const firstRow = canvas.getAllByTestId("dashboard-upstream-account-recent-row")[0];
    const firstRowAction = firstRow?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-row-action"]',
    );
    if (!(firstRowAction instanceof HTMLButtonElement)) {
      throw new Error("expected upstream recent row action button");
    }

    await userEvent.click(firstRowAction);
    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-drawer-state"]')?.textContent,
      ).toContain("invocation:acct-invoke-1");
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "invocation:acct-invoke-1",
    );
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Proves the upstream-account recent identity chip opens the conversation drawer while the surrounding recent row still opens the invocation drawer.",
      },
    },
  },
};

export const UpstreamAccountTabDynamicSeven: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DashboardAccountWindowEvidenceSurface>
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-seven", [
            createPreview({
              id: 9811,
              invokeId: "story-working-seven",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(7)}
        recentPreviewLimit={4}
        upstreamAccountRecentPreviewLimit={7}
      />
    </DashboardAccountWindowEvidenceSurface>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await expect(canvas.getByText("最近 7 条调用")).toBeInTheDocument();
    await expect(canvas.getByText("story-account-7")).toBeInTheDocument();
    await expect(
      canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip"),
    ).toHaveLength(7);
    const identityChips = canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip");
    await expect(new Set(identityChips.map((chip) => chip.className)).size).toBeGreaterThan(3);
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Medium dynamic recent invocation window showing seven account rows and stable short conversation identity chips with discrete helper tones.",
      },
    },
  },
};

export const UpstreamAccountTabMaxSixteen: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DashboardAccountWindowEvidenceSurface>
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-sixteen", [
            createPreview({
              id: 9821,
              invokeId: "story-working-sixteen",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse(16)}
        recentPreviewLimit={4}
        upstreamAccountRecentPreviewLimit={16}
      />
    </DashboardAccountWindowEvidenceSurface>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    await expect(canvas.getByText("最近 16 条调用")).toBeInTheDocument();
    await expect(canvas.getByText("story-account-16")).toBeInTheDocument();
    await expect(
      canvas.getAllByTestId("dashboard-upstream-account-recent-identity-chip"),
    ).toHaveLength(16);
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Upper clamp state for the upstream-account recent invocation list, keeping the dense account card scannable at sixteen rows.",
      },
    },
  },
};

export const DrawerInteractionFlow: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }

    await userEvent.click(currentSlot);

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="dashboard-invocation-detail-drawer"]'),
      ).not.toBeNull();
    });

    const drawerAccountButton = document.body.querySelector(
      '[data-testid="dashboard-invocation-detail-drawer"] button[title="pool-account-77@example.com"]',
    );
    if (!(drawerAccountButton instanceof HTMLButtonElement)) {
      throw new Error("missing drawer account button");
    }

    await userEvent.click(drawerAccountButton);

    await waitFor(() => {
      expect(document.body.querySelector('[data-testid="story-account-drawer"]')).not.toBeNull();
    });
  },
};

export const ConversationSelectionOff: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
};

export const ConversationSelectionOn: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await enableConversationSelectionMode(canvasElement);
  },
};

export const ConversationBulkPanelOpen: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await selectConversationForBulkActions(canvasElement);
  },
};

export const ConversationBulkRouteBindDialog: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await selectConversationForBulkActions(canvasElement);
    const routeBindButton = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-route-bind-button"]',
    );
    if (!(routeBindButton instanceof HTMLButtonElement)) {
      throw new Error("missing route bind button");
    }

    await userEvent.click(routeBindButton);
    await waitFor(() => {
      expect(
        canvasElement.ownerDocument.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
        ),
      ).not.toBeNull();
    });
  },
};

export const ConversationBulkRouteBindDialogRecentTargets: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => (
    <BulkSelectionStorySurface {...args} recentTargets={bulkSelectionStoryRecentTargets} />
  ),
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await selectConversationForBulkActions(canvasElement);
    const routeBindButton = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-route-bind-button"]',
    );
    if (!(routeBindButton instanceof HTMLButtonElement)) {
      throw new Error("missing route bind button");
    }

    await userEvent.click(routeBindButton);
    await waitFor(() => {
      expect(
        canvasElement.ownerDocument.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-dialog"]',
        ),
      ).not.toBeNull();
      expect(
        canvasElement.ownerDocument.body.querySelectorAll(
          '[data-testid="dashboard-working-conversations-route-bind-recent-chip"]',
        ).length,
      ).toBe(5);
      expect(
        canvasElement.ownerDocument.body.querySelector(
          '[role="combobox"][aria-label="批量账号绑定目标"]',
        )?.textContent,
      ).toContain("growth.6vv4@relay.example · CIII");
    });
  },
};

export const ConversationBulkRouteBindDialogRecentTargetsMobile: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => (
    <BulkSelectionStorySurface {...args} recentTargets={bulkSelectionStoryRecentTargets} />
  ),
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    await selectConversationForBulkActions(canvasElement);
    const routeBindButton = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-route-bind-button"]',
    );
    if (!(routeBindButton instanceof HTMLButtonElement)) {
      throw new Error("missing route bind button");
    }

    await userEvent.click(routeBindButton);
    await waitFor(() => {
      expect(
        canvasElement.ownerDocument.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-recents"]',
        ),
      ).not.toBeNull();
      expect(
        canvasElement.ownerDocument.body.querySelectorAll(
          '[data-testid="dashboard-working-conversations-route-bind-recent-chip"]',
        ).length,
      ).toBeGreaterThan(0);
      expect(
        canvasElement.ownerDocument.body.querySelector(
          '[data-testid="dashboard-working-conversations-route-bind-recent-overflow-chip"]',
        ),
      ).toBeNull();
      expect(
        canvasElement.ownerDocument.body.querySelectorAll(
          '[data-testid="dashboard-working-conversations-route-bind-recent-chip"]',
        ).length,
      ).toBeLessThan(bulkSelectionStoryRecentTargets.length);
    });
  },
};

export const ConversationBulkClearConfirm: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} theme="vibe-dark" />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await waitFor(() => {
      expect(canvasElement.ownerDocument.documentElement.getAttribute("data-theme")).toBe(
        "vibe-dark",
      );
      expect(canvasElement.ownerDocument.body.getAttribute("data-theme")).toBe("vibe-dark");
    });

    const { footer, callout } = await openBulkClearBindingDialog(canvasElement);

    const footerBg =
      canvasElement.ownerDocument.defaultView?.getComputedStyle(footer).backgroundColor ?? "";
    const calloutBg =
      canvasElement.ownerDocument.defaultView?.getComputedStyle(callout).backgroundColor ?? "";

    expect(readPerceptualChannel(footerBg)).toBeLessThan(0.5);
    expect(readPerceptualChannel(calloutBg)).toBeLessThan(0.5);
  },
};

export const ConversationBulkClearConfirmLight: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} theme="vibe-light" />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await waitFor(() => {
      expect(canvasElement.ownerDocument.documentElement.getAttribute("data-theme")).toBe(
        "vibe-light",
      );
      expect(canvasElement.ownerDocument.body.getAttribute("data-theme")).toBe("vibe-light");
    });

    const { footer, callout } = await openBulkClearBindingDialog(canvasElement);

    const footerBg =
      canvasElement.ownerDocument.defaultView?.getComputedStyle(footer).backgroundColor ?? "";
    const calloutBg =
      canvasElement.ownerDocument.defaultView?.getComputedStyle(callout).backgroundColor ?? "";

    expect(readPerceptualChannel(footerBg)).toBeGreaterThan(0.8);
    expect(readPerceptualChannel(calloutBg)).toBeGreaterThan(0.8);
  },
};

export const ConversationBulkFastModeChooser: Story = {
  args: bulkSelectionStoryArgs,
  render: (args) => <BulkSelectionStorySurface {...args} />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
  },
  play: async ({ canvasElement }) => {
    await selectConversationForBulkActions(canvasElement);
    const fastModeButton = canvasElement.ownerDocument.body.querySelector(
      '[data-testid="dashboard-working-conversations-fast-mode-button"]',
    );
    if (!(fastModeButton instanceof HTMLButtonElement)) {
      throw new Error("missing fast mode button");
    }

    await userEvent.click(fastModeButton);
    await waitFor(() => {
      expect(
        canvasElement.ownerDocument.body.querySelector(
          '[data-testid="dashboard-working-conversations-fast-mode-popover"]',
        ),
      ).not.toBeNull();
    });
  },
};

export const StateGallery: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(wideDesktopResponse),
    isLoading: false,
    error: null,
  },
};

export const LoadingState: Story = {
  args: {
    activeRange: "today",
    cards: [],
    totalMatched: 0,
    isLoading: true,
    error: null,
  },
};

export const EmptyState: Story = {
  args: {
    activeRange: "today",
    cards: [],
    totalMatched: 0,
    isLoading: false,
    error: null,
  },
};

export const ErrorState: Story = {
  args: {
    activeRange: "today",
    cards: [],
    totalMatched: 0,
    isLoading: false,
    error: "Request failed: 503 working conversations snapshot unavailable",
  },
};

export const Mobile390: Story = {
  tags: ["test"],
  args: {
    activeRange: "today",
    cards: buildCards(wideDesktopResponse),
    totalMatched: wideDesktopResponse.conversations.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    docs: {
      description: {
        story:
          "Mobile viewport keeps the working-conversations section in a single column while preserving the compact header and three-slot summary hierarchy.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const activeWorkspaceTab = canvasElement.querySelector('[role="tab"][aria-selected="true"]');
    if (!(activeWorkspaceTab instanceof HTMLElement)) {
      throw new Error("missing active workspace view tab");
    }
    await expect(activeWorkspaceTab).toHaveTextContent(/对话|Conversations/);
    await expect(
      canvasElement.querySelectorAll('[data-testid="dashboard-working-conversation-card"]'),
    ).not.toHaveLength(0);
    await expect(
      canvasElement.querySelectorAll('[data-testid="dashboard-upstream-account-card"]'),
    ).toHaveLength(0);
    const controls = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversations-controls"]',
    );
    if (!(controls instanceof HTMLElement)) {
      throw new Error("missing workspace controls");
    }
    await expect(controls.className).toContain("flex-col");
    await expect(controls.querySelector('[role="tablist"]')?.className).toContain("w-full");
  },
};

export const Mobile393: Story = {
  ...Mobile390,
  parameters: {
    viewport: { defaultViewport: "mobile393" },
    docs: {
      description: {
        story:
          "393x852 responsive acceptance viewport keeps the working-conversations section in a single column without horizontal overflow.",
      },
    },
  },
};

export const WideDesktop1660: Story = {
  tags: ["test"],
  args: {
    activeRange: "today",
    cards: buildCards(wideDesktopResponse),
    totalMatched: wideDesktopResponse.conversations.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Wide desktop state gallery proving the 1660px shell renders the working conversations section in four columns without horizontal overflow.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const controls = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversations-controls"]',
    );
    if (!(controls instanceof HTMLElement)) {
      throw new Error("missing workspace controls");
    }
    await expect(controls.firstElementChild?.getAttribute("role")).toBe("tablist");
    await expect(controls.children.item(1)?.getAttribute("data-testid")).toBe(
      "dashboard-working-conversations-actions",
    );
    const responseTimes = Array.from(
      canvasElement.querySelectorAll('[data-testid="dashboard-compact-latency-response"]'),
    ).map((element) => element.textContent);
    await expect(responseTimes).toContain("--");
  },
};

export const FourCardParallelThreeSlotProof: Story = {
  tags: ["test"],
  args: {
    activeRange: "today",
    cards: buildCards(fourCardParallelThreeSlotProofResponse),
    totalMatched: fourCardParallelThreeSlotProofResponse.conversations.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Direct proof for the compact invocation layout: the native 1660px four-card row includes a card with current, previous, and earlier real invocations.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const cards = Array.from(
      canvasElement.querySelectorAll('[data-testid="dashboard-working-conversation-card"]'),
    );
    await expect(cards).toHaveLength(4);
    const targetCard = cards.find((card) =>
      card.textContent?.includes("gpt-5.4-long-context-preview"),
    );
    if (!(targetCard instanceof HTMLElement)) {
      throw new Error("missing three-real-invocation proof card");
    }
    await expect(
      targetCard.querySelectorAll('[data-testid="dashboard-working-conversation-slot"]'),
    ).toHaveLength(3);
    await expect(
      targetCard.querySelectorAll('[data-testid="dashboard-working-conversation-placeholder"]'),
    ).toHaveLength(0);
  },
};

export const VirtualizedLargeDataset: Story = {
  args: {
    activeRange: "today",
    cards: virtualizedLargeDatasetCards,
    totalMatched: virtualizedLargeDatasetCards.length,
    isLoading: false,
    error: null,
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Large loaded working set proving the section keeps the DOM virtualized instead of mounting every card at once.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const container = await canvas.findByTestId("dashboard-working-conversations-grid");
    const storyWindow = canvasElement.ownerDocument.defaultView;
    if (!storyWindow) {
      throw new Error("missing story window");
    }

    const scrollTarget = container.getBoundingClientRect().top + storyWindow.scrollY + 1_600;
    storyWindow.scrollTo({ top: scrollTarget });

    await waitFor(() => {
      const renderedCards = container.querySelectorAll(
        '[data-testid="dashboard-working-conversation-card"]',
      ).length;
      expect(renderedCards).toBeGreaterThan(0);
      expect(renderedCards).toBeLessThan(virtualizedLargeDatasetCards.length);
    });

    expect(container.className).not.toContain("overflow-auto");
    expect(container.className).not.toContain("max-h-[72vh]");
  },
};

export const HeadInsertAnchorCompensation: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <HeadInsertAnchorStory />,
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Auto-prepends a fresh head card after the list has been scrolled, and the existing viewport anchor should stay visually pinned instead of jumping downward.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const container = await canvas.findByTestId("dashboard-working-conversations-grid");
    const storyWindow = canvasElement.ownerDocument.defaultView;
    if (!storyWindow) {
      throw new Error("missing story window");
    }

    const scrollTarget = container.getBoundingClientRect().top + storyWindow.scrollY + 1_600;
    storyWindow.scrollTo({ top: scrollTarget });
    storyWindow.dispatchEvent(new Event("scroll"));

    let anchorCard: HTMLElement | undefined;
    await waitFor(() => {
      anchorCard = Array.from(
        container.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        ),
      ).find((candidate) => candidate.getBoundingClientRect().height > 0);
      expect(anchorCard).toBeDefined();
    });

    const anchorSequenceId = anchorCard?.dataset.conversationSequenceId ?? "";
    const containerTopBoundary = Math.max(0, container.getBoundingClientRect().top);
    const anchorTop = (anchorCard?.getBoundingClientRect().top ?? 0) - containerTopBoundary;

    await waitFor(() => {
      expect(canvas.getByTestId("story-head-insert-status")).toHaveTextContent(
        "prepended:pck-anchor-new-head",
      );
    });

    await waitFor(() => {
      const nextAnchor = Array.from(
        container.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        ),
      ).find((candidate) => candidate.dataset.conversationSequenceId === anchorSequenceId);
      expect(nextAnchor).toBeDefined();
      const nextTop = (nextAnchor?.getBoundingClientRect().top ?? 0) - containerTopBoundary;
      expect(Math.abs(nextTop - anchorTop)).toBeLessThanOrEqual(12);
    });
  },
};

export const PhaseSummary: Story = {
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "desktop1660x900" },
  },
  args: {
    activeRange: "today",
    cards: buildCards(
      createResponse([
        createConversation(
          "pck-phase-summary-mixed",
          [
            createPreview({
              id: 91_001,
              invokeId: "phase-summary-mixed-requesting",
              occurredAt: createRelativeStoryIso(-5_000),
              status: "running",
              livePhase: "requesting",
            }),
          ],
          { inFlightPhaseCounts: { queued: 1, requesting: 3, responding: 0 } },
        ),
        createConversation(
          "pck-phase-summary-responding",
          [
            createPreview({
              id: 91_002,
              invokeId: "phase-summary-responding",
              occurredAt: "2026-04-04T10:04:57Z",
              status: "running",
              livePhase: "responding",
            }),
          ],
          { inFlightPhaseCounts: { queued: 0, requesting: 1, responding: 1 } },
        ),
      ]),
    ),
    isLoading: false,
    error: null,
  },
  render: (args) => (
    <div data-visual-evidence-surface="dashboard-phase-summary" className="bg-base-200 px-8 py-8">
      <div data-visual-evidence-target="dashboard-phase-summary-cards">
        <DashboardWorkingConversationsSection {...args} />
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const summaries = canvasElement.querySelectorAll(
      '[data-testid="dashboard-working-conversation-phase-summary"]',
    );
    await expect(summaries).toHaveLength(2);
    const mixed = Array.from(summaries).find((summary) =>
      summary.querySelector('[data-phase="requesting"]')?.textContent?.includes("3"),
    );
    if (!(mixed instanceof HTMLElement)) throw new Error("missing mixed phase summary");
    await expect(mixed.querySelector('[data-phase="queued"]')).toBeInTheDocument();
    await expect(mixed.querySelector('[data-phase="requesting"]')).toBeInTheDocument();
    await expect(mixed.querySelector('[data-phase="responding"]')).toBeNull();
    await expect(mixed.querySelector('[data-phase="requesting"]')?.textContent).toContain("3");
    const coexist = Array.from(summaries).find(
      (summary) =>
        summary.querySelector('[data-phase="responding"]') != null &&
        !summary.querySelector('[data-phase="requesting"]')?.textContent?.includes("3"),
    );
    if (!(coexist instanceof HTMLElement)) throw new Error("missing coexist phase summary");
    await expect(coexist.querySelector('[data-phase="requesting"]')).toBeInTheDocument();
    await expect(coexist.querySelector('[data-phase="responding"]')).toBeInTheDocument();
  },
};

export const CreatedAtDescendingOrder: Story = {
  args: {
    activeRange: "today",
    cards: createdAtDescendingOrderCards,
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const cards = await canvas.findAllByTestId("dashboard-working-conversation-card");
    expect(cards.map((card) => card.getAttribute("data-conversation-sequence-id"))).toEqual(
      createdAtDescendingOrderKeys.map(getStorySequenceIdForPromptCacheKey),
    );
  },
};

export const UpstreamAccountSortDescendingOrder: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account-sort-ordering", [
          createPreview({
            id: 98_201,
            invokeId: "story-working-upstream-sort-ordering",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "completed",
            upstreamAccountId: 102,
            upstreamAccountName: "Pool High",
          }),
        ]),
      ])}
      upstreamAccountActivity={upstreamAccountSortOrderingResponse}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);

    const readOrder = () =>
      Array.from(
        canvasElement.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-upstream-account-card"]',
        ),
      ).map((card) => card.getAttribute("data-account-key"));

    await waitFor(() => {
      expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);
    });

    const sortButton = canvas.getByTestId("dashboard-workspace-sort-button");
    await userEvent.click(sortButton);
    await waitFor(() => {
      expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);
    });

    await userEvent.click(sortButton);
    await waitFor(() => {
      expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);
    });

    await userEvent.click(sortButton);
    await waitFor(() => {
      expect(readOrder()).toEqual(["assigned-high", "assigned-mid", "unassigned"]);
    });
  },
};
