import type {
  ApiInvocation,
  InvocationPhaseCounts,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationManualBinding,
  PromptCacheConversationsResponse,
} from "./api";
import { EMPTY_INVOCATION_PHASE_COUNTS, resolveInvocationLivePhase } from "./invocationPhase";
import { resolveInvocationDisplayStatus } from "./invocationStatus";
import { buildInvocationFromPromptCachePreview } from "./promptCacheLive";

export const DASHBOARD_WORKING_CONVERSATIONS_LIMIT = 20;
export const DASHBOARD_WORKING_CONVERSATIONS_ACTIVITY_MINUTES = 5;
export const DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE = 20;
export const DASHBOARD_WORKING_CONVERSATIONS_SELECTION = {
  mode: "activityWindow",
  activityMinutes: DASHBOARD_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
} as const;

export type DashboardWorkingConversationTone =
  | "running"
  | "pending"
  | "success"
  | "warning"
  | "error"
  | "neutral";

export interface DashboardWorkingConversationInvocationModel {
  preview: PromptCacheConversationInvocationPreview;
  record: ApiInvocation;
  displayStatus: string;
  livePhase: ApiInvocation["livePhase"];
  occurredAtEpoch: number | null;
  isInFlight: boolean;
  isTerminal: boolean;
  tone: DashboardWorkingConversationTone;
}

export interface DashboardWorkingConversationCardModel {
  promptCacheKey: string;
  normalizedPromptCacheKey: string;
  conversationId: string;
  manualBinding?: PromptCacheConversationManualBinding | null;
  createdAtEpoch: number | null;
  currentInvocation: DashboardWorkingConversationInvocationModel;
  previousInvocation: DashboardWorkingConversationInvocationModel | null;
  earlierInvocation: DashboardWorkingConversationInvocationModel | null;
  hasPreviousPlaceholder: boolean;
  hasEarlierPlaceholder: boolean;
  sortAnchorEpoch: number;
  lastTerminalAtEpoch: number | null;
  lastInFlightAtEpoch: number | null;
  tone: DashboardWorkingConversationTone;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  inFlightPhaseCounts: InvocationPhaseCounts;
}

export interface DashboardWorkingConversationInvocationSelection {
  slotKind: "current" | "previous" | "earlier";
  conversationId: string | null;
  promptCacheKey: string;
  invocation: DashboardWorkingConversationInvocationModel;
}

interface DashboardWorkingConversationMapOptions {
  limit?: number;
}

type PendingSequenceCardModel = DashboardWorkingConversationCardModel;

function normalizePromptCacheKey(value: string) {
  return value.trim();
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isNaN(epoch) ? null : epoch;
}

function isInFlightStatus(status: string) {
  return status === "running" || status === "pending";
}

export function buildDashboardWorkingConversationInvocationModel(
  preview: PromptCacheConversationInvocationPreview,
): DashboardWorkingConversationInvocationModel {
  const record = buildInvocationFromPromptCachePreview(preview);
  const displayStatus = resolveInvocationDisplayStatus(record) || "unknown";
  const livePhase = resolveInvocationLivePhase(record);
  const normalizedStatus = displayStatus.trim().toLowerCase();
  const isInFlight = isInFlightStatus(normalizedStatus);
  const tone: DashboardWorkingConversationTone =
    normalizedStatus === "running"
      ? "running"
      : normalizedStatus === "pending"
        ? "pending"
        : normalizedStatus === "success" || normalizedStatus === "completed"
          ? "success"
          : normalizedStatus === "warning_success"
            ? "warning"
            : normalizedStatus.startsWith("http_4")
              ? "warning"
              : normalizedStatus.startsWith("http_") ||
                  normalizedStatus === "failed" ||
                  normalizedStatus === "interrupted"
                ? "error"
                : "neutral";

  return {
    preview,
    record,
    displayStatus,
    livePhase,
    occurredAtEpoch: parseEpoch(preview.occurredAt),
    isInFlight,
    isTerminal: !isInFlight,
    tone,
  };
}

function sortInvocationsByOccurredAtDesc(
  left: DashboardWorkingConversationInvocationModel,
  right: DashboardWorkingConversationInvocationModel,
) {
  const leftEpoch = left.occurredAtEpoch ?? Number.MIN_SAFE_INTEGER;
  const rightEpoch = right.occurredAtEpoch ?? Number.MIN_SAFE_INTEGER;
  if (leftEpoch !== rightEpoch) return rightEpoch - leftEpoch;
  return right.preview.invokeId.localeCompare(left.preview.invokeId);
}

function buildPendingCardModel(
  conversation: PromptCacheConversation,
  rangeStartEpoch: number,
): PendingSequenceCardModel | null {
  const normalizedPromptCacheKey = normalizePromptCacheKey(conversation.promptCacheKey);
  const conversationId = conversation.conversationId?.trim();
  if (!normalizedPromptCacheKey || !conversationId) return null;

  const invocations = conversation.recentInvocations
    .map(buildDashboardWorkingConversationInvocationModel)
    .sort(sortInvocationsByOccurredAtDesc);
  const currentInvocation = invocations[0];
  if (!currentInvocation) return null;

  const previousInvocation = invocations[1] ?? null;
  const earlierInvocation = invocations[2] ?? null;
  const lastTerminalAtEpoch =
    parseEpoch(conversation.lastTerminalAt) ??
    invocations.find(
      (invocation) =>
        invocation.isTerminal &&
        invocation.occurredAtEpoch != null &&
        invocation.occurredAtEpoch >= rangeStartEpoch,
    )?.occurredAtEpoch ??
    null;
  const lastInFlightAtEpoch =
    parseEpoch(conversation.lastInFlightAt) ??
    invocations.find((invocation) => invocation.isInFlight)?.occurredAtEpoch ??
    null;
  const sortAnchorEpoch = Math.max(
    lastTerminalAtEpoch ?? Number.MIN_SAFE_INTEGER,
    lastInFlightAtEpoch ?? Number.MIN_SAFE_INTEGER,
    currentInvocation.occurredAtEpoch ?? Number.MIN_SAFE_INTEGER,
  );

  return {
    promptCacheKey: conversation.promptCacheKey,
    normalizedPromptCacheKey,
    conversationId,
    manualBinding: conversation.manualBinding ?? null,
    createdAtEpoch: parseEpoch(conversation.firstInvocationAt ?? conversation.createdAt),
    currentInvocation,
    previousInvocation,
    earlierInvocation,
    hasPreviousPlaceholder: previousInvocation == null,
    hasEarlierPlaceholder: earlierInvocation == null,
    sortAnchorEpoch,
    lastTerminalAtEpoch,
    lastInFlightAtEpoch,
    tone: currentInvocation.tone,
    requestCount: conversation.requestCount,
    totalTokens: conversation.totalTokens,
    totalCost: conversation.totalCost,
    inFlightPhaseCounts: conversation.inFlightPhaseCounts ?? EMPTY_INVOCATION_PHASE_COUNTS,
  };
}

function compareDashboardWorkingConversationDisplayOrder(
  left: PendingSequenceCardModel,
  right: PendingSequenceCardModel,
) {
  const leftCreatedAtEpoch = left.createdAtEpoch ?? Number.MIN_SAFE_INTEGER;
  const rightCreatedAtEpoch = right.createdAtEpoch ?? Number.MIN_SAFE_INTEGER;
  if (leftCreatedAtEpoch !== rightCreatedAtEpoch) {
    return rightCreatedAtEpoch - leftCreatedAtEpoch;
  }

  return right.normalizedPromptCacheKey.localeCompare(left.normalizedPromptCacheKey);
}

function compareDashboardWorkingConversationVisibleSetOrder(
  left: PendingSequenceCardModel,
  right: PendingSequenceCardModel,
) {
  if (left.sortAnchorEpoch !== right.sortAnchorEpoch) {
    return right.sortAnchorEpoch - left.sortAnchorEpoch;
  }

  return compareDashboardWorkingConversationDisplayOrder(left, right);
}

export function mapPromptCacheConversationsToDashboardCards(
  response: PromptCacheConversationsResponse | null,
  options: DashboardWorkingConversationMapOptions = {},
) {
  if (!response) return [] satisfies DashboardWorkingConversationCardModel[];

  const rangeStartEpoch = parseEpoch(response.rangeStart) ?? Number.MIN_SAFE_INTEGER;
  const visibleSetCards = response.conversations
    .map((conversation) => buildPendingCardModel(conversation, rangeStartEpoch))
    .filter((card): card is PendingSequenceCardModel => card != null)
    .sort(compareDashboardWorkingConversationVisibleSetOrder);

  if (typeof options.limit === "number" && Number.isFinite(options.limit)) {
    visibleSetCards.splice(options.limit);
  }

  return visibleSetCards.sort(compareDashboardWorkingConversationDisplayOrder);
}
