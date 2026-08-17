import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingLiveResponse } from "../../lib/api";
import { ModelRoutingLivePanel } from "./ModelRoutingLivePanel";

const data: ModelRoutingLiveResponse = {
  generatedAt: "2026-08-16T04:00:00.000Z",
  groups: [
    {
      model: "gpt-5.5",
      accounts: [
        {
          accountId: 11,
          accountDisplayName: "API Key #11",
          model: "gpt-5.5",
          state: "available",
          priority: "normal",
          failureCount: 0,
          changedAt: "2026-08-16T03:59:20.000Z",
          lastSeenAt: "2026-08-16T03:59:20.000Z",
        },
        {
          accountId: 12,
          accountDisplayName: "API Key #12",
          model: "gpt-5.5",
          state: "cooling_down",
          priority: "excluded",
          failureCount: 3,
          changedAt: "2026-08-16T03:57:11.000Z",
          lastSeenAt: "2026-08-16T03:57:11.000Z",
          cooldownUntil: "2026-08-16T04:15:00.000Z",
          cacheConcurrencyLimit: 1,
          cacheLastHitRatePercent: 4.2,
          probeRequired: true,
        },
      ],
    },
  ],
  records: [
    {
      id: "attempt:1",
      kind: "attempt",
      occurredAt: "2026-08-16T03:59:20.000Z",
      accountId: 11,
      accountDisplayName: "API Key #11",
      model: "gpt-5.5",
      attemptId: "attempt-001",
      invokeId: "invoke-001",
      attemptIndex: 2,
      sameAccountRetryIndex: 1,
      status: "success",
      httpStatus: 200,
      totalLatencyMs: 821,
      reasonCode: "probe_passed",
      modelRouteStateBefore: "cooling_down",
      modelRouteStateAfter: "available",
      routingSelectionAudit: {
        selectedAccountId: 11,
        selectedAccountName: "API Key #11",
        eligibleCandidateCount: 2,
        winnerReasonCode: "lowest_effective_load",
        excludedCandidates: [],
      },
    },
  ],
};

const meta = {
  title: "Live/ModelRoutingLivePanel",
  component: ModelRoutingLivePanel,
  tags: ["autodocs", "test"],
  parameters: { layout: "fullscreen" },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="bg-neutral p-8 text-neutral-content [&_.section-description]:!text-neutral-content [&_.section-title]:!text-neutral-content sm:p-12">
          <div className="mx-auto max-w-[1440px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  args: {
    data,
    isLoading: false,
    window: "24h",
    onWindowChange: () => undefined,
    onModelChange: () => undefined,
    onStateChange: () => undefined,
    onOpenAccount: () => undefined,
    onOpenInvocation: () => undefined,
    onRefresh: () => undefined,
  },
} satisfies Meta<typeof ModelRoutingLivePanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const RecoveryAttempt: Story = {
  globals: {
    viewport: { value: "desktop1440", isRotated: false },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-routing-gantt-gpt-5.5")).toBeVisible();
    const controls = canvas.getByTestId("model-routing-live-controls");
    await expect(controls).toBeVisible();
    const refresh = within(controls).getByRole("button", { name: "刷新" });
    const state = within(controls).getByLabelText("路由状态");
    const model = within(controls).getByLabelText("模型");
    const timeWindow = within(controls).getByText("15m");
    await expect(refresh).toBeVisible();
    await expect(state).toBeVisible();
    await expect(model).toBeVisible();
    await expect(timeWindow).toBeVisible();
    await expect(
      refresh.compareDocumentPosition(state) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    await expect(state.compareDocumentPosition(model) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(
      0,
    );
    await expect(
      model.compareDocumentPosition(timeWindow) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    await expect(canvas.getByText("未知")).toBeVisible();
    await expect(canvas.getByText("请求尝试")).toBeVisible();
  },
};

export const RecoveryAttemptMobile: Story = {
  ...RecoveryAttempt,
  globals: {
    viewport: { value: "mobile393", isRotated: false },
  },
};

export const Empty: Story = {
  args: { data: { generatedAt: "2026-08-16T04:00:00.000Z", groups: [], records: [] } },
};
