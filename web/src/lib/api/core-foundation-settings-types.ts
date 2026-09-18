import type {
  ApiInvocation,
  BlockedBindingConstraintSource,
  BlockedBindingDiagnostic,
  InvocationLivePhase,
  InvocationPhaseCounts,
  PoolRoutingSelectionAudit,
} from "./core-foundation-record-types";
import type {
  CodexImagegenRewriteMode,
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingTimeoutSettings,
} from "./core-upstream";
export interface VersionResponse {
  backend: string;
  frontend: string;
}

export interface PricingEntry {
  model: string;
  inputPer1m: number;
  outputPer1m: number;
  cacheInputPer1m?: number | null;
  cacheReadPer1m?: number | null;
  cacheWritePer1m?: number | null;
  reasoningPer1m?: number | null;
  source: string;
}

export interface PricingSettings {
  catalogVersion: string;
  entries: PricingEntry[];
}

export type ProxyFastModeRewriteMode = "disabled" | "fill_missing" | "force_priority";

export interface ProxySettings {
  hijackEnabled: boolean;
  mergeUpstreamEnabled: boolean;
  fastModeRewriteMode: ProxyFastModeRewriteMode;
  upstream429MaxRetries: number;
  websocketEnabled: boolean;
  upstreamWebsocketDefaultEnabled: boolean;
  requestBodyLoggingEnabled: boolean;
  responseBodyLoggingEnabled: boolean;
  encryptedSessionOwnerRoutingEnabled: boolean;
  defaultHijackEnabled: boolean;
  models: string[];
  imageModels?: string[];
  enabledModels: string[];
}

export interface ForwardProxyWindowStats {
  attempts: number;
  successRate?: number;
  avgLatencyMs?: number;
}

export interface ForwardProxyNodeStats {
  oneMinute: ForwardProxyWindowStats;
  fifteenMinutes: ForwardProxyWindowStats;
  oneHour: ForwardProxyWindowStats;
  oneDay: ForwardProxyWindowStats;
  sevenDays: ForwardProxyWindowStats;
}

export interface ForwardProxyNode {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  stats: ForwardProxyNodeStats;
}

export interface ForwardProxyBindingNode {
  key: string;
  aliasKeys?: string[];
  source: string;
  displayName: string;
  protocolLabel: string;
  egressIp?: string | null;
  egressIpCheckedAt?: string | null;
  egressIpProvider?: string | null;
  egressIpError?: string | null;
  egressIpErrorAt?: string | null;
  penalized: boolean;
  selectable: boolean;
  last24h: ForwardProxyHourlyBucket[];
}

export interface ForwardProxySettings {
  proxyUrls: string[];
  subscriptionUrls: string[];
  subscriptionUpdateIntervalSecs: number;
  nodes: ForwardProxyNode[];
}

export interface ForwardProxyRefreshSubscriptionsResult {
  forwardProxy: ForwardProxySettings;
  subscriptionCount: number;
  addedNodeCount: number;
  refreshedAt: string;
}

export interface ForwardProxyLatencyTargetResult {
  ok: boolean;
  latencyMs?: number;
  ip?: string;
  httpStatus?: number;
  error?: string;
}

export interface ForwardProxyLatencyTestNodeProgress {
  key: string;
  displayName: string;
  round: number;
  totalRounds: number;
  completedRounds: number;
  successCount: number;
  attemptCount: number;
  averageLatencyMs?: number;
  egressIp: ForwardProxyLatencyTargetResult;
  oauthUpstream: ForwardProxyLatencyTargetResult;
  codexResponses: ForwardProxyLatencyTargetResult;
  allTargetsOk: boolean;
  failedTargets: string[];
  done: boolean;
  timedOut: boolean;
  message: string;
}

export interface ForwardProxyLatencyTestStreamEvent {
  kind: "progress" | "completed";
  node: ForwardProxyLatencyTestNodeProgress;
}

export interface ForwardProxyHourlyBucket {
  bucketStart: string;
  bucketEnd: string;
  successCount: number;
  failureCount: number;
}

export interface ForwardProxyWeightBucket {
  bucketStart: string;
  bucketEnd: string;
  sampleCount: number;
  minWeight: number;
  maxWeight: number;
  avgWeight: number;
  lastWeight: number;
}

export interface ForwardProxyLiveNode {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  stats: ForwardProxyNodeStats;
  last24h: ForwardProxyHourlyBucket[];
  weight24h: ForwardProxyWeightBucket[];
}

export interface ForwardProxyLiveStatsResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  nodes: ForwardProxyLiveNode[];
}

export interface ForwardProxyTimeseriesNode {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  buckets: ForwardProxyHourlyBucket[];
  weightBuckets: ForwardProxyWeightBucket[];
}

export interface ForwardProxyTimeseriesResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  effectiveBucket: string;
  availableBuckets: string[];
  nodes: ForwardProxyTimeseriesNode[];
}

export interface ConversationRequestPoint {
  occurredAt: string;
  status: string;
  isSuccess: boolean;
  outcome?: ConversationRequestOutcome | null;
  requestTokens: number;
  cumulativeTokens: number;
}

export type ConversationRequestOutcome = "success" | "failure" | "neutral" | "in_flight";

export type PromptCacheConversationRequestPoint = ConversationRequestPoint;

export type StickyKeyConversationRequestPoint = ConversationRequestPoint;

export interface PromptCacheConversationUpstreamAccount {
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  lastActivityAt: string;
}

export type PromptCacheConversationManualBindingKind = "group" | "upstreamAccount";

export interface PromptCacheConversationManualBinding {
  bindingKind: PromptCacheConversationManualBindingKind;
  groupName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
}

export interface PromptCacheConversationInvocationPreview {
  id: number;
  invokeId: string;
  promptCacheKey?: string | null;
  occurredAt: string;
  status: string;
  livePhase?: InvocationLivePhase | null;
  failureClass: Exclude<ApiInvocation["failureClass"], undefined> | null;
  routeMode: string | null;
  model: string | null;
  requestModel?: string | null;
  responseModel?: string | null;
  totalTokens: number;
  cost: number | null;
  proxyDisplayName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  upstreamAccountPlanType?: string | null;
  endpoint: string | null;
  compactionRequestKind?: ApiInvocation["compactionRequestKind"];
  compactionResponseKind?: ApiInvocation["compactionResponseKind"];
  imageIntent?: ApiInvocation["imageIntent"];
  source?: ApiInvocation["source"];
  inputTokens?: ApiInvocation["inputTokens"];
  outputTokens?: ApiInvocation["outputTokens"];
  cacheInputTokens?: ApiInvocation["cacheInputTokens"];
  cacheWriteTokens?: ApiInvocation["cacheWriteTokens"];
  costInput?: ApiInvocation["costInput"];
  costCacheWrite?: ApiInvocation["costCacheWrite"];
  costCacheRead?: ApiInvocation["costCacheRead"];
  costOutput?: ApiInvocation["costOutput"];
  costReasoning?: ApiInvocation["costReasoning"];
  reasoningTokens?: ApiInvocation["reasoningTokens"];
  reasoningEffort?: ApiInvocation["reasoningEffort"];
  errorMessage?: ApiInvocation["errorMessage"];
  downstreamStatusCode?: ApiInvocation["downstreamStatusCode"];
  downstreamErrorMessage?: ApiInvocation["downstreamErrorMessage"];
  failureKind?: ApiInvocation["failureKind"];
  isActionable?: ApiInvocation["isActionable"];
  responseContentEncoding?: ApiInvocation["responseContentEncoding"];
  requestCompressionAlgorithm?: ApiInvocation["requestCompressionAlgorithm"];
  transport?: ApiInvocation["transport"];
  requestedServiceTier?: ApiInvocation["requestedServiceTier"];
  serviceTier?: ApiInvocation["serviceTier"];
  billingServiceTier?: ApiInvocation["billingServiceTier"];
  tReqReadMs?: ApiInvocation["tReqReadMs"];
  tReqParseMs?: ApiInvocation["tReqParseMs"];
  tUpstreamConnectMs?: ApiInvocation["tUpstreamConnectMs"];
  tUpstreamTtfbMs?: ApiInvocation["tUpstreamTtfbMs"];
  firstTokenMs?: ApiInvocation["firstTokenMs"];
  tUpstreamStreamMs?: ApiInvocation["tUpstreamStreamMs"];
  tRespParseMs?: ApiInvocation["tRespParseMs"];
  tPersistMs?: ApiInvocation["tPersistMs"];
  tTotalMs?: ApiInvocation["tTotalMs"];
  blockedBinding?: ApiInvocation["blockedBinding"];
}

export interface PromptCacheConversation {
  promptCacheKey: string;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  createdAt: string;
  lastActivityAt: string;
  lastTerminalAt?: string | null;
  lastInFlightAt?: string | null;
  inFlightPhaseCounts?: InvocationPhaseCounts | null;
  cursor?: string | null;
  hasEncryptedSessionOwner: boolean;
  encryptedOwnerAccountId?: number | null;
  encryptedOwnerAccountName?: string | null;
  encryptedOwnerGroupName?: string | null;
  manualBinding?: PromptCacheConversationManualBinding | null;
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  recentInvocations: PromptCacheConversationInvocationPreview[];
  last24hRequests: PromptCacheConversationRequestPoint[];
  blockedBinding?: BlockedBindingDiagnostic | null;
}

export type PromptCacheConversationBindingKind = "none" | "group" | "upstreamAccount";
export type PromptCacheConversationRewriteMode =
  | "force_remove"
  | "keep_original"
  | "fill_missing"
  | "force_add";

export interface PromptCacheConversationBindingResponse {
  promptCacheKey: string;
  bindingKind: PromptCacheConversationBindingKind;
  groupName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  hasEncryptedSessionOwner: boolean;
  encryptedOwnerAccountId: number | null;
  encryptedOwnerAccountName: string | null;
  encryptedOwnerGroupName: string | null;
  stickyRoutes?: PromptCacheConversationStickyRoute[];
  timeouts: PoolRoutingTimeoutSettings;
  timeoutFieldSources: EffectiveRoutingTimeoutFieldSources;
  allowSwitchUpstream?: boolean | null;
  fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
  imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
  codexImagegenRewriteMode?: CodexImagegenRewriteMode | null;
  availableModels?: string[] | null;
  availableModelsMode?: "allowlist" | "denylist" | null;
  forwardProxyKey?: string | null;
  forwardProxyKeys?: string[];
  policyFieldSources?: {
    allowSwitchUpstream: EffectiveRoutingRuleSource;
    fastModeRewriteMode: EffectiveRoutingRuleSource;
    imageToolRewriteMode: EffectiveRoutingRuleSource;
    codexImagegenRewriteMode?: EffectiveRoutingRuleSource;
    availableModels: EffectiveRoutingRuleSource;
    availableModelsMode?: EffectiveRoutingRuleSource;
    forwardProxyKey: EffectiveRoutingRuleSource;
  };
  updatedAt: string | null;
}

export interface PromptCacheConversationStickyRoute {
  modelKey: string | null;
  upstreamAccountId: number;
  upstreamAccountName: string | null;
  createdAt: string;
  updatedAt: string;
  lastSeenAt: string;
}

export type PromptCacheConversationOperationInfoType =
  | "routing"
  | "forwardProxy"
  | "requestRewrite";
export type PromptCacheConversationOperationOrigin =
  | "detailDrawer"
  | "dashboardBulk"
  | "systemAuto";
export type PromptCacheConversationOperationAction =
  | "manualBindingUpdated"
  | "bindingCleared"
  | "affinityReset"
  | "stickyTargetChanged"
  | "stickyTargetCleared"
  | "stickyMutationSuppressed"
  | "groupBindingPromoted"
  | "conversationPolicyUpdated";

export interface PromptCacheConversationOperationBindingSnapshot {
  bindingKind: PromptCacheConversationBindingKind;
  groupName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
}

export interface PromptCacheConversationOperationStickySnapshot {
  upstreamAccountId: number;
  upstreamAccountName: string | null;
}

export interface PromptCacheConversationOperationRoutingContext {
  reasonCode: string;
  routingSource: "stickyReuse" | "freshAssignment" | string | null;
  routingSelectionAudit?: PoolRoutingSelectionAudit | null;
  httpStatus: number | null;
  triggerAttemptId: string | null;
  causingAttemptId: string | null;
  causingHttpStatus: number | null;
}

export interface PromptCacheConversationOperationRoutingScope {
  kind: "all" | "model";
  modelKey: string | null;
  requestModel: string | null;
}

export interface PromptCacheConversationOperationStickyTransition {
  modelKey: string | null;
  before: PromptCacheConversationOperationStickySnapshot | null;
  after: PromptCacheConversationOperationStickySnapshot | null;
}

export interface PromptCacheConversationOperationEvent {
  id: number;
  promptCacheKey: string;
  action: PromptCacheConversationOperationAction;
  origin: PromptCacheConversationOperationOrigin;
  infoTypes: PromptCacheConversationOperationInfoType[];
  occurredAt: string;
  headline: string;
  changedFields: string[];
  bindingBefore: PromptCacheConversationOperationBindingSnapshot | null;
  bindingAfter: PromptCacheConversationOperationBindingSnapshot | null;
  stickyBefore: PromptCacheConversationOperationStickySnapshot | null;
  stickyAfter: PromptCacheConversationOperationStickySnapshot | null;
  invokeId: string | null;
  routingContext?: PromptCacheConversationOperationRoutingContext | null;
  routingScope?: PromptCacheConversationOperationRoutingScope | null;
  stickyTransitions?: PromptCacheConversationOperationStickyTransition[];
}

export interface PromptCacheConversationOperationEventListResponse {
  items: PromptCacheConversationOperationEvent[];
  total: number;
  page: number;
  pageSize: number;
  routingModelFacets?: string[];
}

export type PromptCacheConversationBindingTimeoutPatch = {
  responsesFirstByteTimeoutSecs?: number | null;
  compactFirstByteTimeoutSecs?: number | null;
  imageFirstByteTimeoutSecs?: number | null;
  responsesStreamTimeoutSecs?: number | null;
  compactStreamTimeoutSecs?: number | null;
};

export type UpdatePromptCacheConversationBindingPayload =
  | {
      bindingKind: "none";
      timeouts?: PromptCacheConversationBindingTimeoutPatch;
      allowSwitchUpstream?: boolean | null;
      fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
      imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
      codexImagegenRewriteMode?: CodexImagegenRewriteMode | null;
      availableModels?: string[] | null;
      availableModelsMode?: "allowlist" | "denylist" | null;
      forwardProxyKey?: string | null;
      forwardProxyKeys?: string[] | null;
    }
  | {
      bindingKind: "group";
      groupName: string;
      timeouts?: PromptCacheConversationBindingTimeoutPatch;
      allowSwitchUpstream?: boolean | null;
      fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
      imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
      codexImagegenRewriteMode?: CodexImagegenRewriteMode | null;
      availableModels?: string[] | null;
      availableModelsMode?: "allowlist" | "denylist" | null;
      forwardProxyKey?: string | null;
      forwardProxyKeys?: string[] | null;
    }
  | {
      bindingKind: "upstreamAccount";
      upstreamAccountId: number;
      timeouts?: PromptCacheConversationBindingTimeoutPatch;
      allowSwitchUpstream?: boolean | null;
      fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
      imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
      codexImagegenRewriteMode?: CodexImagegenRewriteMode | null;
      availableModels?: string[] | null;
      availableModelsMode?: "allowlist" | "denylist" | null;
      forwardProxyKey?: string | null;
      forwardProxyKeys?: string[] | null;
    };

export type BulkPromptCacheConversationBindActionPayload =
  | {
      action: "bind";
      bindingKind: "none";
      promptCacheKeys: string[];
    }
  | {
      action: "bind";
      bindingKind: "group";
      groupName: string;
      promptCacheKeys: string[];
    }
  | {
      action: "bind";
      bindingKind: "upstreamAccount";
      upstreamAccountId: number;
      promptCacheKeys: string[];
    };

export type BulkPromptCacheConversationBindingActionPayload =
  | BulkPromptCacheConversationBindActionPayload
  | {
      action: "clearAndResetAffinity";
      promptCacheKeys: string[];
    }
  | {
      action: "setFastModeRewriteMode";
      fastModeRewriteMode: PromptCacheConversationRewriteMode;
      promptCacheKeys: string[];
    };

export interface BulkPromptCacheConversationBindingItemResponse {
  promptCacheKey: string;
  ok: boolean;
  error: string | null;
  binding: PromptCacheConversationBindingResponse | null;
}

export interface BulkPromptCacheConversationBindingActionResponse {
  action: "bind" | "clearAndResetAffinity" | "setFastModeRewriteMode";
  totalRequested: number;
  totalSucceeded: number;
  totalFailed: number;
  items: BulkPromptCacheConversationBindingItemResponse[];
}

export interface FetchPromptCacheConversationOperationEventsQuery {
  page?: number;
  pageSize?: number;
  infoType?: PromptCacheConversationOperationInfoType;
  routingScope?: "all" | "model";
  routingModel?: string;
}

export type PromptCacheConversationSelectionMode = "count" | "activityWindow";
export type PromptCacheConversationDetailLevel = "full" | "compact";

export type PromptCacheConversationImplicitFilterKind = "inactiveOutside24h" | "cappedTo50";

export interface PromptCacheConversationImplicitFilter {
  kind: PromptCacheConversationImplicitFilterKind | null;
  filteredCount: number;
}

export type PromptCacheConversationSelection =
  | { mode: "count"; limit: number }
  | { mode: "activityWindow"; activityHours: number }
  | { mode: "activityWindow"; activityMinutes: number };

export interface PromptCacheConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
  snapshotAt?: string | null;
  selectionMode: PromptCacheConversationSelectionMode;
  selectedLimit: number | null;
  selectedActivityHours: number | null;
  selectedActivityMinutes?: number | null;
  implicitFilter: PromptCacheConversationImplicitFilter;
  totalMatched?: number | null;
  hasMore?: boolean;
  nextCursor?: string | null;
  conversations: PromptCacheConversation[];
}

export interface PromptCacheConversationPageQuery {
  pageSize?: number;
  cursor?: string | null;
  snapshotAt?: string | null;
  detail?: PromptCacheConversationDetailLevel;
  recentInvocationLimit?: number;
  blockedBindingUpstreamAccountId?: number | null;
  blockedBindingConstraintSource?: BlockedBindingConstraintSource | null;
  signal?: AbortSignal;
}

export type StickyKeyConversationSelectionMode = PromptCacheConversationSelectionMode;

export type StickyKeyConversationImplicitFilterKind = PromptCacheConversationImplicitFilterKind;

export interface StickyKeyConversationImplicitFilter {
  kind: StickyKeyConversationImplicitFilterKind | null;
  filteredCount: number;
}

export type StickyKeyConversationSelection =
  | { mode: "count"; limit: number }
  | { mode: "activityWindow"; activityHours: number };

export type StickyKeyConversationInvocationPreview = PromptCacheConversationInvocationPreview;

export interface StickyKeyConversation {
  stickyKey: string;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  createdAt: string;
  lastActivityAt: string;
  recentInvocations: StickyKeyConversationInvocationPreview[];
  last24hRequests: StickyKeyConversationRequestPoint[];
}

export interface UpstreamStickyConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
  selectionMode: StickyKeyConversationSelectionMode;
  selectedLimit: number | null;
  selectedActivityHours: number | null;
  implicitFilter: StickyKeyConversationImplicitFilter;
  conversations: StickyKeyConversation[];
}

export type ForwardProxyValidationKind = "proxyUrl" | "subscriptionUrl";

export interface ForwardProxyValidationResult {
  ok: boolean;
  message: string;
  normalizedValue?: string;
  discoveredNodes?: number;
  latencyMs?: number;
}
