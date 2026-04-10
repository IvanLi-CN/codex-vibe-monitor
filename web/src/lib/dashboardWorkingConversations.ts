import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "./api";
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
  occurredAtEpoch: number | null;
  isInFlight: boolean;
  isTerminal: boolean;
  tone: DashboardWorkingConversationTone;
}

export interface DashboardWorkingConversationCardModel {
  promptCacheKey: string;
  normalizedPromptCacheKey: string;
  conversationSequenceId: string;
  createdAtEpoch: number | null;
  currentInvocation: DashboardWorkingConversationInvocationModel;
  previousInvocation: DashboardWorkingConversationInvocationModel | null;
  hasPreviousPlaceholder: boolean;
  sortAnchorEpoch: number;
  lastTerminalAtEpoch: number | null;
  lastInFlightAtEpoch: number | null;
  tone: DashboardWorkingConversationTone;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
}

export interface DashboardWorkingConversationInvocationSelection {
  slotKind: "current" | "previous";
  conversationSequenceId: string;
  promptCacheKey: string;
  invocation: DashboardWorkingConversationInvocationModel;
}

interface DashboardWorkingConversationSequenceOptions {
  hashFn?: (value: string) => string;
  collisionHashFn?: (value: string) => string;
}

interface DashboardWorkingConversationMapOptions extends DashboardWorkingConversationSequenceOptions {
  limit?: number;
}

type PendingSequenceCardModel = Omit<
  DashboardWorkingConversationCardModel,
  "conversationSequenceId"
>;

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

function normalizeHash(
  value: string | null | undefined,
  minimumLength: number,
) {
  const compact = (value ?? "")
    .trim()
    .replace(/[^a-z0-9]/gi, "")
    .toUpperCase();
  if (compact.length >= minimumLength) return compact;
  return compact.padEnd(minimumLength, "0");
}

export function formatDashboardWorkingConversationSequenceId(value: string) {
  const normalized = value.trim();
  if (!normalized) return normalized;
  return normalized.replace(/^WC-/i, "");
}

export function hashDashboardWorkingConversationKey(value: string) {
  let hash = 0x811c9dc5;
  for (const character of value) {
    hash ^= character.charCodeAt(0);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0").toUpperCase();
}

function buildInvocationModel(
  preview: PromptCacheConversationInvocationPreview,
): DashboardWorkingConversationInvocationModel {
  const record = buildInvocationFromPromptCachePreview(preview);
  const displayStatus = resolveInvocationDisplayStatus(record) || "unknown";
  const normalizedStatus = displayStatus.trim().toLowerCase();
  const isInFlight = isInFlightStatus(normalizedStatus);
  const tone: DashboardWorkingConversationTone =
    normalizedStatus === "running"
      ? "running"
      : normalizedStatus === "pending"
        ? "pending"
        : normalizedStatus === "success" || normalizedStatus === "completed"
          ? "success"
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
  const normalizedPromptCacheKey = normalizePromptCacheKey(
    conversation.promptCacheKey,
  );
  if (!normalizedPromptCacheKey) return null;

  const invocations = conversation.recentInvocations
    .map(buildInvocationModel)
    .sort(sortInvocationsByOccurredAtDesc);
  const currentInvocation = invocations[0];
  if (!currentInvocation) return null;

  const previousInvocation = invocations[1] ?? null;
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
    createdAtEpoch: parseEpoch(conversation.createdAt),
    currentInvocation,
    previousInvocation,
    hasPreviousPlaceholder: previousInvocation == null,
    sortAnchorEpoch,
    lastTerminalAtEpoch,
    lastInFlightAtEpoch,
    tone: currentInvocation.tone,
    requestCount: conversation.requestCount,
    totalTokens: conversation.totalTokens,
    totalCost: conversation.totalCost,
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

  return right.normalizedPromptCacheKey.localeCompare(
    left.normalizedPromptCacheKey,
  );
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

  const rangeStartEpoch =
    parseEpoch(response.rangeStart) ?? Number.MIN_SAFE_INTEGER;
  const sortedCards = response.conversations
    .map((conversation) => buildPendingCardModel(conversation, rangeStartEpoch))
    .filter((card): card is PendingSequenceCardModel => card != null)
    .sort(compareDashboardWorkingConversationVisibleSetOrder);

  if (typeof options.limit === "number" && Number.isFinite(options.limit)) {
    sortedCards.splice(options.limit);
  }

  const hashFn = options.hashFn ?? hashDashboardWorkingConversationKey;
  const collisionHashFn =
    options.collisionHashFn ??
    ((value: string) =>
      hashDashboardWorkingConversationKey(`collision:${value}`));

  const primaryBuckets = new Map<string, PendingSequenceCardModel[]>();
  for (const card of sortedCards) {
    const primaryHash = normalizeHash(
      hashFn(card.normalizedPromptCacheKey),
      6,
    ).slice(0, 6);
    const bucket = primaryBuckets.get(primaryHash) ?? [];
    bucket.push(card);
    primaryBuckets.set(primaryHash, bucket);
  }

  return sortedCards.map<DashboardWorkingConversationCardModel>((card) => {
    const primaryHash = normalizeHash(
      hashFn(card.normalizedPromptCacheKey),
      6,
    ).slice(0, 6);
    const colliders = primaryBuckets.get(primaryHash) ?? [card];
    let conversationSequenceId = `WC-${primaryHash}`;

    if (colliders.length > 1) {
      const secondaryHash = normalizeHash(
        collisionHashFn(card.normalizedPromptCacheKey),
        2,
      ).slice(0, 2);
      const secondaryBuckets = new Map<string, PendingSequenceCardModel[]>();
      for (const collider of colliders) {
        const suffix = normalizeHash(
          collisionHashFn(collider.normalizedPromptCacheKey),
          2,
        ).slice(0, 2);
        const bucket = secondaryBuckets.get(suffix) ?? [];
        bucket.push(collider);
        secondaryBuckets.set(suffix, bucket);
      }

      const duplicateSuffixCards = secondaryBuckets.get(secondaryHash) ?? [
        card,
      ];
      if (duplicateSuffixCards.length === 1) {
        conversationSequenceId = `WC-${primaryHash}-${secondaryHash}`;
      } else {
        const collisionIndex = duplicateSuffixCards
          .slice()
          .sort((left, right) =>
            left.normalizedPromptCacheKey.localeCompare(
              right.normalizedPromptCacheKey,
            ),
          )
          .findIndex(
            (candidate) =>
              candidate.normalizedPromptCacheKey ===
              card.normalizedPromptCacheKey,
          );
        const fallbackSuffix = `${secondaryHash}${(collisionIndex + 1)
          .toString(36)
          .toUpperCase()}`;
        conversationSequenceId = `WC-${primaryHash}-${fallbackSuffix}`;
      }
    }

    return {
      ...card,
      conversationSequenceId,
    };
  });
}
