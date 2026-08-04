import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider, useTranslation } from "../../i18n";
import type { ApiPoolUpstreamRequestAttempt } from "../../lib/api";
import { PoolAttemptRecordCard } from "./PoolAttemptRecordCard";

const freshAssignmentAttempt: ApiPoolUpstreamRequestAttempt = {
  attemptId: "SELECTAUDIT1",
  invokeId: "selection-audit-invoke",
  occurredAt: "2026-08-03T07:38:27Z",
  endpoint: "/v1/responses",
  stickyKey: "prompt-cache-selection-audit-key",
  routingSource: "freshAssignment",
  routingSelectionAudit: {
    selectedAccountId: 2890,
    selectedAccountName: "dzw",
    eligibleCandidateCount: 1,
    winnerReasonCode: "onlyEligibleCandidate",
    comparedAccountId: null,
    comparedAccountName: null,
    excludedCandidates: [
      {
        accountId: 2805,
        accountName: "CIII",
        reasonCode: "modelNotAllowed",
      },
    ],
  },
  upstreamAccountId: 2890,
  upstreamAccountName: "dzw",
  attemptIndex: 1,
  distinctAccountIndex: 1,
  sameAccountRetryIndex: 0,
  startedAt: "2026-08-03T07:38:27Z",
  finishedAt: "2026-08-03T07:38:28Z",
  status: "success",
  phase: "completed",
  httpStatus: 200,
  createdAt: "2026-08-03T07:38:27Z",
};

function StoryCard() {
  const { t } = useTranslation();
  return (
    <div className="max-w-3xl bg-base-200 p-6">
      <PoolAttemptRecordCard
        attempt={freshAssignmentAttempt}
        proxyDisplay={{ value: "Direct", title: "Direct", resolved: true }}
        t={t}
        testId="fresh-assignment-routing-decision"
      />
    </div>
  );
}

const meta = {
  title: "Invocations/PoolAttemptRecordCard",
  component: StoryCard,
  tags: ["autodocs"],
  decorators: [
    (Story) => (
      <I18nProvider>
        <Story />
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof StoryCard>;

export default meta;

type Story = StoryObj<typeof meta>;

export const FreshAssignmentRoutingDecision: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("pool-attempt-routing-selection-audit")).toBeVisible();
    await expect(
      canvas.getByText(/dzw 是唯一合格候选|dzw was the only eligible candidate/i),
    ).toBeVisible();
    await expect(
      canvas.getByText(/CIII 不允许当前请求模型|CIII did not allow the requested model/i),
    ).toBeVisible();
  },
};
