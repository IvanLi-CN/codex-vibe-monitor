import type {
  ApiInvocation,
  StatsResponse,
  UpstreamAccountActivityAccount,
  UpstreamAccountActivityResponse,
} from "./api";
import { buildPromptCachePreviewFromInvocation } from "./promptCacheLive";

export const DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS = 1_000;

interface DashboardRecordContribution {
  fingerprint: string;
  accountId: number | null;
  requestCount: number;
  terminalCount: number;
  successCount: number;
  failureCount: number;
  totalCost: number;
  totalTokens: number;
  successTokens: number;
  failureTokens: number;
  nonSuccessCost: number;
  nonSuccessTokens: number;
  inFlightCount: number;
  retryCount: number;
}

export interface DashboardRecordPatchState {
  records: Map<string, DashboardRecordContribution>;
  summaryHydratedAtMs: number | null;
  summaryUnknownInFlightCount: number;
  summaryUnknownRetryCount: number;
  accountHydratedAtMsById: Map<number, number>;
  accountUnknownInFlightCountById: Map<number, number>;
  accountUnknownRetryCountById: Map<number, number>;
}

export function createDashboardRecordPatchState(): DashboardRecordPatchState {
  return {
    records: new Map(),
    summaryHydratedAtMs: null,
    summaryUnknownInFlightCount: 0,
    summaryUnknownRetryCount: 0,
    accountHydratedAtMsById: new Map(),
    accountUnknownInFlightCountById: new Map(),
    accountUnknownRetryCountById: new Map(),
  };
}

export function clearDashboardRecordPatchState(
  patchState: DashboardRecordPatchState,
) {
  patchState.records.clear();
  patchState.summaryHydratedAtMs = null;
  patchState.summaryUnknownInFlightCount = 0;
  patchState.summaryUnknownRetryCount = 0;
  patchState.accountHydratedAtMsById.clear();
  patchState.accountUnknownInFlightCountById.clear();
  patchState.accountUnknownRetryCountById.clear();
}

export function seedDashboardSummaryPatchState(
  patchState: DashboardRecordPatchState,
  summary: StatsResponse,
  hydratedAtMs = Date.now(),
) {
  patchState.records.clear();
  patchState.summaryHydratedAtMs = hydratedAtMs;
  patchState.summaryUnknownInFlightCount = finiteNonNegative(
    summary.inProgressConversationCount,
  );
  patchState.summaryUnknownRetryCount = finiteNonNegative(
    summary.inProgressRetryConversationCount,
  );
}

export function seedUpstreamAccountActivityPatchState(
  patchState: DashboardRecordPatchState,
  response: UpstreamAccountActivityResponse,
  hydratedAtMs = Date.now(),
) {
  patchState.records.clear();
  patchState.accountHydratedAtMsById.clear();
  patchState.accountUnknownInFlightCountById.clear();
  patchState.accountUnknownRetryCountById.clear();

  for (const account of response.accounts) {
    let seededInFlight = 0;
    let seededRetry = 0;
    for (const preview of account.recentInvocations) {
      const record = preview as ApiInvocation;
      const contribution = contributionForRecord(record, {
        countLiveUsage: true,
      });
      patchState.records.set(dashboardRecordIdentity(record), contribution);
      seededInFlight += contribution.inFlightCount;
      seededRetry += contribution.retryCount;
    }

    patchState.accountHydratedAtMsById.set(account.upstreamAccountId, hydratedAtMs);
    patchState.accountUnknownInFlightCountById.set(
      account.upstreamAccountId,
      Math.max(0, finiteNonNegative(account.inProgressInvocationCount) - seededInFlight),
    );
    patchState.accountUnknownRetryCountById.set(
      account.upstreamAccountId,
      Math.max(0, finiteNonNegative(account.retryInvocationCount) - seededRetry),
    );
  }
}

export function dashboardRecordFingerprint(record: ApiInvocation) {
  return [
    record.id,
    record.invokeId,
    record.status ?? "",
    resolveRecordFailureClass(record),
    normalizeOptionalText(record.failureKind) ?? "",
    record.errorMessage?.trim() ?? "",
    record.downstreamErrorMessage?.trim() ?? "",
    record.occurredAt,
    record.totalTokens ?? "",
    record.cost ?? "",
    record.upstreamAccountId ?? "",
    record.poolAttemptCount ?? "",
    record.poolAttemptTerminalReason ?? "",
  ].join("::");
}

export function dashboardRecordIdentity(record: ApiInvocation) {
  const invokeId = record.invokeId.trim();
  return invokeId || `${record.id}::${record.occurredAt}`;
}

export function dashboardRecordIsInFlight(record: Pick<ApiInvocation, "status">) {
  const normalized = record.status?.trim().toLowerCase() ?? "";
  return normalized === "running" || normalized === "pending";
}

export function dashboardRecordIsSuccess(record: Pick<ApiInvocation, "status">) {
  const normalized = record.status?.trim().toLowerCase() ?? "";
  return (
    normalized === "success" ||
    normalized === "completed" ||
    normalized === "http_200"
  );
}

export function dashboardRecordIsTerminal(record: Pick<ApiInvocation, "status">) {
  const normalized = record.status?.trim().toLowerCase() ?? "";
  return Boolean(normalized) && !dashboardRecordIsInFlight(record);
}

function finiteNonNegative(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, value)
    : 0;
}

function stableRecordSort(
  left: Pick<ApiInvocation, "occurredAt" | "id">,
  right: Pick<ApiInvocation, "occurredAt" | "id">,
) {
  const occurredCompare = right.occurredAt.localeCompare(left.occurredAt);
  if (occurredCompare !== 0) return occurredCompare;
  return right.id - left.id;
}

export function recordFallsWithinTimeRange(
  record: Pick<ApiInvocation, "occurredAt">,
  rangeStart: string,
  rangeEnd: string,
) {
  const occurredMs = Date.parse(record.occurredAt ?? "");
  const startMs = Date.parse(rangeStart);
  const endMs = Date.parse(rangeEnd);
  return (
    Number.isFinite(occurredMs) &&
    Number.isFinite(startMs) &&
    Number.isFinite(endMs) &&
    occurredMs >= startMs &&
    occurredMs < endMs
  );
}

function recordFallsWithinAccountActivityRange(
  record: Pick<ApiInvocation, "occurredAt">,
  response: Pick<UpstreamAccountActivityResponse, "range" | "rangeStart" | "rangeEnd">,
) {
  if (response.range !== "today") {
    return recordFallsWithinTimeRange(record, response.rangeStart, response.rangeEnd);
  }

  const occurredMs = Date.parse(record.occurredAt ?? "");
  const startMs = Date.parse(response.rangeStart);
  return (
    Number.isFinite(occurredMs) &&
    Number.isFinite(startMs) &&
    occurredMs >= startMs
  );
}

export function filterDashboardRecordsForLocalDay(
  records: ApiInvocation[],
  now = Date.now(),
) {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  const end = new Date(start);
  end.setDate(end.getDate() + 1);
  return records.filter((record) =>
    recordFallsWithinTimeRange(
      record,
      start.toISOString(),
      end.toISOString(),
    ),
  );
}

function recordAccountId(record: Pick<ApiInvocation, "upstreamAccountId">) {
  return typeof record.upstreamAccountId === "number" &&
    Number.isFinite(record.upstreamAccountId)
    ? record.upstreamAccountId
    : null;
}

function recordMayHaveExistedAtHydration(
  record: Pick<ApiInvocation, "createdAt">,
  hydratedAtMs: number | null | undefined,
) {
  if (hydratedAtMs == null) return false;
  const createdAtMs = Date.parse(record.createdAt ?? "");
  return Number.isFinite(createdAtMs) && createdAtMs <= hydratedAtMs;
}

function contributionForRecord(
  record: ApiInvocation,
  options: { countLiveUsage?: boolean } = {},
): DashboardRecordContribution {
  const isInFlight = dashboardRecordIsInFlight(record);
  const isTerminal = dashboardRecordIsTerminal(record);
  const resolvedFailureClass = resolveRecordFailureClass(record);
  const isSuccess = dashboardRecordIsSuccess(record) && resolvedFailureClass === "none";
  const countsTowardFailure =
    isTerminal && resolvedFailureClass !== "none";
  const countsTowardNonSuccess =
    isTerminal && resolvedFailureClass !== "none";
  const recordCost = finiteNonNegative(record.cost);
  const recordTokens = finiteNonNegative(record.totalTokens);
  const countsUsage = isTerminal || options.countLiveUsage === true;
  const terminalReason = record.poolAttemptTerminalReason?.trim().toLowerCase() ?? "";
  const retryCount =
    isInFlight &&
    (terminalReason.includes("retry") || finiteNonNegative(record.poolAttemptCount) > 1)
      ? 1
      : 0;

  return {
    fingerprint: dashboardRecordFingerprint(record),
    accountId: recordAccountId(record),
    requestCount: countsUsage ? 1 : 0,
    terminalCount: isTerminal ? 1 : 0,
    successCount: isTerminal && isSuccess ? 1 : 0,
    failureCount: countsTowardFailure ? 1 : 0,
    totalCost: countsUsage ? recordCost : 0,
    totalTokens: countsUsage ? recordTokens : 0,
    successTokens: isTerminal && isSuccess ? recordTokens : 0,
    failureTokens: countsTowardFailure ? recordTokens : 0,
    nonSuccessCost: countsTowardNonSuccess ? recordCost : 0,
    nonSuccessTokens: countsTowardNonSuccess ? recordTokens : 0,
    inFlightCount: isInFlight ? 1 : 0,
    retryCount,
  };
}

function normalizeOptionalText(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase() ?? "";
  return normalized.length > 0 ? normalized : null;
}

function hasErrorText(record: Pick<ApiInvocation, "errorMessage" | "downstreamErrorMessage">) {
  return (
    (record.errorMessage?.trim().length ?? 0) > 0 ||
    (record.downstreamErrorMessage?.trim().length ?? 0) > 0
  );
}

function resolveRecordFailureClass(
  record: Pick<
    ApiInvocation,
    "status" | "failureClass" | "failureKind" | "errorMessage" | "downstreamErrorMessage"
  >,
): "none" | "service_failure" | "client_failure" | "client_abort" {
  const storedFailureClass = normalizeOptionalText(record.failureClass);
  if (
    storedFailureClass === "service_failure" ||
    storedFailureClass === "client_failure" ||
    storedFailureClass === "client_abort"
  ) {
    return storedFailureClass;
  }

  const status = record.status?.trim().toLowerCase() ?? "";
  const failureKind = normalizeOptionalText(record.failureKind);
  const hasError = hasErrorText(record);
  const hasFailureKind = failureKind != null && failureKind !== "none";

  if (
    (status === "success" || status === "completed" || status === "http_200") &&
    !hasError &&
    !hasFailureKind
  ) {
    return "none";
  }
  if ((status === "running" || status === "pending") && !hasError) {
    return "none";
  }
  if (status === "" && !hasError && !hasFailureKind) {
    return "none";
  }

  if (
    failureKind === "downstream_closed" ||
    record.errorMessage?.toLowerCase().includes("downstream closed while streaming upstream response") ||
    record.downstreamErrorMessage?.toLowerCase().includes("downstream closed while streaming upstream response")
  ) {
    return "client_abort";
  }
  if (status === "http_429" || failureKind === "upstream_http_429") {
    return "service_failure";
  }
  if (
    failureKind === "request_body_stream_error_client_closed" ||
    failureKind === "invalid_api_key" ||
    failureKind === "api_key_not_found" ||
    failureKind === "api_key_missing" ||
    (status.startsWith("http_4") && status !== "http_429")
  ) {
    return "client_failure";
  }
  return "service_failure";
}

function subtractContribution(
  left: DashboardRecordContribution,
  right?: DashboardRecordContribution,
): DashboardRecordContribution {
  return {
    ...left,
    requestCount: left.requestCount - (right?.requestCount ?? 0),
    terminalCount: left.terminalCount - (right?.terminalCount ?? 0),
    successCount: left.successCount - (right?.successCount ?? 0),
    failureCount: left.failureCount - (right?.failureCount ?? 0),
    totalCost: left.totalCost - (right?.totalCost ?? 0),
    totalTokens: left.totalTokens - (right?.totalTokens ?? 0),
    successTokens: left.successTokens - (right?.successTokens ?? 0),
    failureTokens: left.failureTokens - (right?.failureTokens ?? 0),
    nonSuccessCost: left.nonSuccessCost - (right?.nonSuccessCost ?? 0),
    nonSuccessTokens: left.nonSuccessTokens - (right?.nonSuccessTokens ?? 0),
    inFlightCount: left.inFlightCount - (right?.inFlightCount ?? 0),
    retryCount: left.retryCount - (right?.retryCount ?? 0),
  };
}

function hasContributionDelta(delta: DashboardRecordContribution) {
  return (
    delta.terminalCount !== 0 ||
    delta.requestCount !== 0 ||
    delta.successCount !== 0 ||
    delta.failureCount !== 0 ||
    delta.totalCost !== 0 ||
    delta.totalTokens !== 0 ||
    delta.successTokens !== 0 ||
    delta.failureTokens !== 0 ||
    delta.nonSuccessCost !== 0 ||
    delta.nonSuccessTokens !== 0 ||
    delta.inFlightCount !== 0 ||
    delta.retryCount !== 0
  );
}

function consumeSummaryUnknownInFlight(
  patchState: DashboardRecordPatchState,
  record: ApiInvocation,
  delta: DashboardRecordContribution,
) {
  if (
    dashboardRecordIsInFlight(record) &&
    delta.inFlightCount > 0 &&
    patchState.summaryUnknownInFlightCount > 0 &&
    recordMayHaveExistedAtHydration(record, patchState.summaryHydratedAtMs)
  ) {
    patchState.summaryUnknownInFlightCount -= 1;
    const shouldConsumeRetry =
      delta.retryCount > 0 && patchState.summaryUnknownRetryCount > 0;
    if (shouldConsumeRetry) {
      patchState.summaryUnknownRetryCount -= 1;
    }
    return {
      ...delta,
      inFlightCount: delta.inFlightCount - 1,
      retryCount: delta.retryCount - (shouldConsumeRetry ? 1 : 0),
    };
  }
  if (!dashboardRecordIsTerminal(record) || delta.inFlightCount !== 0) return delta;
  if (patchState.summaryUnknownInFlightCount <= 0) return delta;
  if (!recordMayHaveExistedAtHydration(record, patchState.summaryHydratedAtMs)) {
    return delta;
  }
  patchState.summaryUnknownInFlightCount -= 1;
  const shouldConsumeRetry =
    patchState.summaryUnknownRetryCount > 0 &&
    finiteNonNegative(record.poolAttemptCount) > 1;
  if (shouldConsumeRetry) {
    patchState.summaryUnknownRetryCount -= 1;
  }
  return {
    ...delta,
    requestCount: 0,
    inFlightCount: delta.inFlightCount - 1,
    retryCount: delta.retryCount - (shouldConsumeRetry ? 1 : 0),
  };
}

function recordWasCoveredBySummaryHydration(
  patchState: DashboardRecordPatchState,
  record: ApiInvocation,
  previousContribution?: DashboardRecordContribution,
) {
  if (!dashboardRecordIsTerminal(record)) return false;
  if (previousContribution) return false;
  if (patchState.summaryHydratedAtMs == null) return false;
  if (patchState.summaryUnknownInFlightCount > 0) return false;
  return recordMayHaveExistedAtHydration(record, patchState.summaryHydratedAtMs);
}

function consumeAccountUnknownInFlight(
  patchState: DashboardRecordPatchState,
  record: ApiInvocation,
  delta: DashboardRecordContribution,
  accountId: number,
) {
  const unknownInFlight =
    patchState.accountUnknownInFlightCountById.get(accountId) ?? 0;
  if (
    dashboardRecordIsInFlight(record) &&
    delta.inFlightCount > 0 &&
    unknownInFlight > 0 &&
    recordMayHaveExistedAtHydration(
      record,
      patchState.accountHydratedAtMsById.get(accountId),
    )
  ) {
    patchState.accountUnknownInFlightCountById.set(accountId, unknownInFlight - 1);
    const unknownRetry =
      patchState.accountUnknownRetryCountById.get(accountId) ?? 0;
    const shouldConsumeRetry = delta.retryCount > 0 && unknownRetry > 0;
    if (shouldConsumeRetry) {
      patchState.accountUnknownRetryCountById.set(accountId, unknownRetry - 1);
    }
    return {
      ...delta,
      requestCount: 0,
      totalCost: 0,
      totalTokens: 0,
      inFlightCount: delta.inFlightCount - 1,
      retryCount: delta.retryCount - (shouldConsumeRetry ? 1 : 0),
    };
  }
  if (!dashboardRecordIsTerminal(record) || delta.inFlightCount !== 0) return delta;
  if (unknownInFlight <= 0) return delta;
  if (
    !recordMayHaveExistedAtHydration(
      record,
      patchState.accountHydratedAtMsById.get(accountId),
    )
  ) {
    return delta;
  }
  patchState.accountUnknownInFlightCountById.set(accountId, unknownInFlight - 1);
  const unknownRetry =
    patchState.accountUnknownRetryCountById.get(accountId) ?? 0;
  const shouldConsumeRetry =
    unknownRetry > 0 && finiteNonNegative(record.poolAttemptCount) > 1;
  if (shouldConsumeRetry) {
    patchState.accountUnknownRetryCountById.set(accountId, unknownRetry - 1);
  }
  return {
    ...delta,
    requestCount: 0,
    inFlightCount: delta.inFlightCount - 1,
    retryCount: delta.retryCount - (shouldConsumeRetry ? 1 : 0),
  };
}

function applySummaryContributionDelta(
  current: StatsResponse,
  delta: DashboardRecordContribution,
) {
  return {
    ...current,
    totalCount: Math.max(0, current.totalCount + delta.terminalCount),
    successCount: Math.max(0, current.successCount + delta.successCount),
    failureCount: Math.max(0, current.failureCount + delta.failureCount),
    totalCost: Math.max(0, current.totalCost + delta.totalCost),
    totalTokens: Math.max(0, current.totalTokens + delta.totalTokens),
    inProgressConversationCount:
      current.inProgressConversationCount == null
        ? current.inProgressConversationCount
        : Math.max(0, current.inProgressConversationCount + delta.inFlightCount),
    inProgressRetryConversationCount:
      current.inProgressRetryConversationCount == null
        ? current.inProgressRetryConversationCount
        : Math.max(0, current.inProgressRetryConversationCount + delta.retryCount),
    nonSuccessCost:
      current.nonSuccessCost == null
        ? current.nonSuccessCost
        : Math.max(0, current.nonSuccessCost + delta.nonSuccessCost),
    nonSuccessTokens:
      current.nonSuccessTokens == null
        ? current.nonSuccessTokens
        : Math.max(0, current.nonSuccessTokens + delta.nonSuccessTokens),
  };
}

export function patchDashboardSummaryWithRecords(
  current: StatsResponse | null,
  records: ApiInvocation[],
  patchState: DashboardRecordPatchState = createDashboardRecordPatchState(),
): StatsResponse | null {
  if (!current) return current;

  let nextStats = current;
  let changed = false;

  for (const record of records) {
    const identity = dashboardRecordIdentity(record);
    const nextContribution = contributionForRecord(record);
    const previousContribution = patchState.records.get(identity);
    if (previousContribution?.fingerprint === nextContribution.fingerprint) continue;
    if (recordWasCoveredBySummaryHydration(patchState, record, previousContribution)) {
      patchState.records.set(identity, nextContribution);
      continue;
    }
    patchState.records.set(identity, nextContribution);
    const delta = consumeSummaryUnknownInFlight(
      patchState,
      record,
      subtractContribution(nextContribution, previousContribution),
    );
    if (!hasContributionDelta(delta)) continue;
    nextStats = applySummaryContributionDelta(nextStats, delta);
    changed = true;
  }

  return changed ? nextStats : current;
}

function patchAccountWithRecord(
  account: UpstreamAccountActivityAccount,
  record: ApiInvocation,
  delta: DashboardRecordContribution,
  recentLimit: number,
): UpstreamAccountActivityAccount {
  const preview = buildPromptCachePreviewFromInvocation(record);
  const recentInvocations = [preview, ...account.recentInvocations]
    .sort(stableRecordSort)
    .filter((item, index, items) => {
      const firstIndex = items.findIndex(
        (candidate) =>
          candidate.invokeId === item.invokeId,
      );
      return firstIndex === index;
    })
    .slice(0, recentLimit);

  return {
    ...account,
    requestCount: Math.max(0, account.requestCount + delta.requestCount),
    successCount: Math.max(0, account.successCount + delta.successCount),
    failureCount: Math.max(0, account.failureCount + delta.failureCount),
    nonSuccessCount: Math.max(0, account.nonSuccessCount + delta.failureCount),
    totalTokens: Math.max(0, account.totalTokens + delta.totalTokens),
    successTokens: Math.max(0, account.successTokens + delta.successTokens),
    nonSuccessTokens: Math.max(0, account.nonSuccessTokens + delta.nonSuccessTokens),
    failureTokens: Math.max(0, account.failureTokens + delta.failureTokens),
    failureCost: Math.max(0, account.failureCost + delta.nonSuccessCost),
    totalCost: Math.max(0, account.totalCost + delta.totalCost),
    inProgressInvocationCount:
      account.inProgressInvocationCount == null
        ? account.inProgressInvocationCount
        : Math.max(0, account.inProgressInvocationCount + delta.inFlightCount),
    retryInvocationCount:
      account.retryInvocationCount == null
        ? account.retryInvocationCount
        : Math.max(0, account.retryInvocationCount + delta.retryCount),
    recentInvocations,
  };
}

export function patchUpstreamAccountActivityWithRecords(
  current: UpstreamAccountActivityResponse | null,
  records: ApiInvocation[],
  recentLimit: number,
  patchState: DashboardRecordPatchState = createDashboardRecordPatchState(),
): {
  response: UpstreamAccountActivityResponse | null;
  missedAccountRecord: boolean;
} {
  if (!current) {
    return { response: current, missedAccountRecord: false };
  }
  if (records.length === 0) {
    return { response: current, missedAccountRecord: false };
  }

  const accountsById = new Map(
    current.accounts.map((account) => [account.upstreamAccountId, account]),
  );
  let changed = false;
  let missedAccountRecord = false;

  for (const record of records) {
    if (!recordFallsWithinAccountActivityRange(record, current)) {
      continue;
    }
    const accountId = recordAccountId(record);
    if (accountId == null) continue;
    const account = accountsById.get(accountId);
    if (!account) {
      missedAccountRecord = true;
      continue;
    }
    const identity = dashboardRecordIdentity(record);
    const nextContribution = contributionForRecord(record, {
      countLiveUsage: true,
    });
    const previousContribution = patchState.records.get(identity);
    if (previousContribution?.fingerprint === nextContribution.fingerprint) continue;
    patchState.records.set(identity, nextContribution);
    const delta = consumeAccountUnknownInFlight(
      patchState,
      record,
      subtractContribution(nextContribution, previousContribution),
      accountId,
    );
    accountsById.set(accountId, patchAccountWithRecord(account, record, delta, recentLimit));
    changed = true;
  }

  if (!changed) {
    return { response: current, missedAccountRecord };
  }

  return {
    response: {
      ...current,
      accounts: current.accounts.map(
        (account) => accountsById.get(account.upstreamAccountId) ?? account,
      ),
    },
    missedAccountRecord,
  };
}
