import { Alert } from "../../components/ui/alert";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import { InvocationWorkflowAttemptRecord } from "./InvocationWorkflowDetailPanel.attempt-record";
import {
  type AttemptSection,
  buildConversationShortId,
  type InvocationWorkflowDetailPanelProps,
} from "./InvocationWorkflowDetailPanel.formatters";
import {
  buildInvocationWorkflowDetailMetricsModel,
  buildInvocationWorkflowDetailStatusModel,
  useInvocationWorkflowDetailLoad,
  useInvocationWorkflowDetailSelection,
} from "./InvocationWorkflowDetailPanel.state";
import { InvocationWorkflowDetailPanelView } from "./InvocationWorkflowDetailPanel.view";

export type { AttemptSection };
export { InvocationWorkflowAttemptRecord };

export function InvocationWorkflowDetailPanel({
  record,
  focusedAttemptId = null,
  size = "default",
  onOpenUpstreamAccount,
  hideNonShortIds = false,
}: InvocationWorkflowDetailPanelProps) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isZh = locale === "zh";
  const { detail, isLoading, loadError, requestBodyFetchSeqRef } = useInvocationWorkflowDetailLoad(
    record.id,
  );
  const selection = useInvocationWorkflowDetailSelection(
    detail,
    focusedAttemptId,
    requestBodyFetchSeqRef,
    record.id,
  );
  const {
    openBlockId,
    attemptSection,
    genericSection,
    requestBodyState,
    toggleAttemptSection,
    toggleGenericSection,
  } = selection;

  if (!(record.id > 0)) {
    return (
      <div className="rounded-[1rem] border border-base-300/72 bg-base-100/72 px-4 py-3 text-sm text-base-content/64">
        {isZh ? "调用未落盘" : "Invocation not persisted"}
      </div>
    );
  }

  if (isLoading && !detail) {
    return (
      <div className="flex min-h-40 items-center justify-center rounded-[1rem] border border-base-300/72 bg-base-100/72">
        <Spinner size="lg" />
      </div>
    );
  }

  if (loadError) {
    return (
      <Alert variant="error">
        {isZh ? "详情加载失败：" : "Detail load failed: "}
        {loadError}
      </Alert>
    );
  }

  if (!detail) {
    return null;
  }

  const conversationShortId = buildConversationShortId(detail.hero.promptCacheKey);
  const statusModel = buildInvocationWorkflowDetailStatusModel({
    record,
    detail,
    localeTag,
    isZh,
    onOpenUpstreamAccount,
  });
  const metricsModel = buildInvocationWorkflowDetailMetricsModel({
    record,
    detail,
    localeTag,
    isZh,
    finalStatusMeta: statusModel.finalStatusMeta,
  });
  return (
    <InvocationWorkflowDetailPanelView
      record={record}
      detail={detail}
      localeTag={localeTag}
      isZh={isZh}
      size={size}
      hideNonShortIds={hideNonShortIds}
      conversationShortId={conversationShortId}
      finalStatusMeta={statusModel.finalStatusMeta}
      summaryRows={statusModel.summaryRows}
      heroStatusNotes={statusModel.heroStatusNotes}
      noCandidateAudit={statusModel.noCandidateAudit}
      noCandidateReasonCounts={statusModel.noCandidateReasonCounts}
      snapshotMetrics={metricsModel.snapshotMetrics}
      modelTrailItems={metricsModel.modelTrailItems}
      openBlockId={openBlockId}
      attemptSection={attemptSection}
      genericSection={genericSection}
      requestBodyState={requestBodyState}
      toggleAttemptSection={toggleAttemptSection}
      toggleGenericSection={toggleGenericSection}
    />
  );
}
