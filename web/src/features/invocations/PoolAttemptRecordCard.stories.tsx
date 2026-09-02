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
    eligibleCandidateCount: 2,
    winnerReasonCode: "lowerModelRoutePenalty",
    comparedAccountId: 2805,
    comparedAccountName: "CIII",
    selectedScore: {
      eligibility: "assignable",
      routeBindingFailurePenalty: 0,
      modelRoutePenalty: 0,
      modelRoutePenaltyCode: "normal",
      routingPriorityRank: 0,
      capacityLane: "primary",
      dispatchState: "readyOnOwnedNode",
      secondaryResetProximitySecs: null,
      primaryResetProximitySecs: null,
      scarcityScore: "0.000000",
      effectiveLoad: 0,
      lastSelectedAt: null,
    },
    comparedScore: {
      eligibility: "assignable",
      routeBindingFailurePenalty: 0,
      modelRoutePenalty: 1,
      modelRoutePenaltyCode: "demoted",
      routingPriorityRank: 0,
      capacityLane: "primary",
      dispatchState: "readyOnOwnedNode",
      secondaryResetProximitySecs: null,
      primaryResetProximitySecs: null,
      scarcityScore: "0.000000",
      effectiveLoad: 0,
      lastSelectedAt: null,
    },
    excludedCandidates: [],
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
    <div className="max-w-3xl bg-blue-200 p-6">
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
      canvas.getByText(/在 2 个合格候选中选择了 dzw|dzw was selected from 2 eligible candidate/i),
    ).toBeVisible();
    await expect(canvas.getByTestId("pool-attempt-routing-selection-score")).toHaveTextContent(
      /model-route penalty 0|模型路由惩罚 0/i,
    );
    await expect(
      canvas.getByText(/CIII 评分：模型路由惩罚 1|CIII score: model-route penalty 1/i),
    ).toBeVisible();
  },
};

function RecoveryStoryCard() {
  const { t } = useTranslation();
  return (
    <div data-visual-evidence-surface className="max-w-3xl bg-blue-200 p-6">
      <div data-visual-evidence-target>
        <PoolAttemptRecordCard
          attempt={{
            ...freshAssignmentAttempt,
            attemptId: "RECOVERYAUDIT1",
            upstreamAccountId: 2918,
            upstreamAccountName: "Ciii2",
            routingSource: "priorityHandoff",
            routingSelectionAudit: {
              ...freshAssignmentAttempt.routingSelectionAudit!,
              selectedAccountId: 2918,
              selectedAccountName: "Ciii2",
              winnerReasonCode: "requestDrivenRecoveryAdmission",
              handoffAdmission: {
                decision: "admitted",
                phase: "verifying",
                verificationSuccessCount: 1,
                generation: 8,
                trigger: "modelRouteRecovery",
              },
            },
          }}
          proxyDisplay={{ value: "Direct", title: "Direct", resolved: true }}
          t={t}
          testId="request-driven-recovery-admission"
        />
      </div>
    </div>
  );
}

export const RequestDrivenRecoveryAdmission: Story = {
  render: RecoveryStoryCard,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("request-driven-recovery-admission")).toBeVisible();
    await expect(
      canvas.getByText(/请求驱动的恢复目标|request-driven recovery target/i),
    ).toBeVisible();
    await expect(canvas.getByText(/模型路由恢复|Model-route recovery/i)).toBeVisible();
  },
};

export const HistoricalDecisionWithoutScore: Story = {
  render: HistoricalStoryCard,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("historical-routing-decision")).toBeVisible();
    await expect(
      canvas.getByText(/历史事件未保存候选评分|Candidate score details were not recorded/i),
    ).toBeVisible();
  },
};

function HistoricalStoryCard() {
  const { t } = useTranslation();
  return (
    <div className="max-w-3xl bg-blue-200 p-6">
      <PoolAttemptRecordCard
        attempt={{
          ...freshAssignmentAttempt,
          attemptId: "HISTORICAL1",
          routingSelectionAudit: {
            ...freshAssignmentAttempt.routingSelectionAudit!,
            selectedScore: null,
            comparedScore: null,
          },
        }}
        proxyDisplay={{ value: "Direct", title: "Direct", resolved: true }}
        t={t}
        testId="historical-routing-decision"
      />
    </div>
  );
}
