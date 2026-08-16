import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
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
          accountDisplayName: "Ciii",
          accountGroupName: "Primary",
          model: "gpt-5.5",
          state: "available",
          priority: "normal",
          failureCount: 0,
          lastSeenAt: "2026-08-16T03:59:20.000Z",
        },
        {
          accountId: 12,
          accountDisplayName: "Ciii2",
          accountGroupName: "Fallback",
          model: "gpt-5.5",
          state: "cooling_down",
          priority: "excluded",
          failureCount: 3,
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
      accountDisplayName: "Ciii",
      accountGroupName: "Primary",
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
        selectedAccountName: "Ciii",
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
    window: "1h",
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
    await expect(canvas.getByTestId("model-routing-account-11-gpt-5.5")).toBeVisible();
    const detailsButton = canvas.getByLabelText("展开决策详情");
    await userEvent.click(detailsButton);
    await expect(canvas.getByText("候选比较")).toBeVisible();
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
