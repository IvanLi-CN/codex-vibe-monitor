import type {
  ApiInvocation,
  ApiPoolUpstreamRequestAttempt,
  InvocationPhaseCounts,
  ModelPerformance,
  StatsResponse,
  UsageBreakdown,
} from "./core-foundation-record-types";
import type { PromptCacheConversationInvocationPreview } from "./core-foundation-settings-types";
import type { EffectiveRoutingRule } from "./core-upstream";
export interface UpstreamAccountActivityAccount {
  accountKey?: string;
  upstreamAccountId: number | null;
  displayName: string;
  latestConversationCreatedAt?: string | null;
  lastInvocationAt?: string | null;
  isUnassigned?: boolean;
  groupName?: string | null;
  planType?: string | null;
  enabled?: boolean | null;
  displayStatus?: string | null;
  enableStatus?: string | null;
  workStatus?: string | null;
  healthStatus?: string | null;
  syncState?: string | null;
  lastError?: string | null;
  lastActionReasonMessage?: string | null;
  requestCount: number;
  successCount: number;
  failureCount: number;
  nonSuccessCount: number;
  totalTokens: number;
  successTokens: number;
  nonSuccessTokens: number;
  failureTokens: number;
  failureCost: number;
  totalCost: number;
  usageBreakdown: UsageBreakdown;
  modelPerformance?: ModelPerformance | null;
  cacheHitRate?: number | null;
  tokensPerMinute?: number | null;
  spendRate?: number | null;
  firstByteAvgMs?: number | null;
  firstResponseByteTotalAvgMs?: number | null;
  firstTokenAvgMs?: number | null;
  avgTotalMs?: number | null;
  currentFirstTokenAvgMs?: number | null;
  currentFirstResponseByteTotalAvgMs?: number | null;
  currentAvgTotalMs?: number | null;
  currentAvgResponseMs?: number | null;
  inProgressInvocationCount?: number | null;
  inProgressPhaseCounts?: InvocationPhaseCounts | null;
  retryInvocationCount?: number | null;
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  effectiveRoutingRule: EffectiveRoutingRule;
  recentInvocations: PromptCacheConversationInvocationPreview[];
}

export interface UpstreamAccountActivityResponse {
  range: string;
  rangeStart: string;
  rangeEnd: string;
  routingStateVersion?: RoutingStateVersion | null;
  networkLiveBucket?: DashboardNetworkTimeseriesPoint | null;
  networkRealtimeRate?: DashboardNetworkRealtimeRate | null;
  accounts: UpstreamAccountActivityAccount[];
}

export interface DashboardActivityRateWindow {
  start: string;
  end: string;
  windowMinutes: number;
  mode: string;
}

export interface DashboardActivitySummary {
  stats: StatsResponse;
  tokensPerMinute?: number | null;
  spendRate?: number | null;
  currentFirstResponseByteTotalAvgMs?: number | null;
  currentFirstTokenAvgMs?: number | null;
  currentAvgTotalMs?: number | null;
  currentAvgResponseMs?: number | null;
  modelPerformance?: ModelPerformance | null;
}

export interface DashboardActivityLiveAccount {
  accountKey: string;
  upstreamAccountId: number | null;
  inProgressInvocationCount: number;
  inProgressPhaseCounts: InvocationPhaseCounts;
  retryInvocationCount: number;
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  networkLiveBucket?: DashboardNetworkTimeseriesPoint | null;
}

export interface DashboardActivityLiveSnapshot {
  revision: number;
  generatedAt: string;
  inProgressInvocationCount: number;
  inProgressPhaseCounts: InvocationPhaseCounts;
  retryInvocationCount: number;
  networkLiveBucket?: DashboardNetworkTimeseriesPoint | null;
  networkRealtimeRate?: DashboardNetworkRealtimeRate | null;
  accounts: DashboardActivityLiveAccount[];
}

export interface DashboardActivityResponse {
  range: string;
  rangeStart: string;
  rangeEnd: string;
  snapshotId: number;
  routingStateVersion?: RoutingStateVersion | null;
  liveRevision?: number;
  rateWindow: DashboardActivityRateWindow;
  summary: DashboardActivitySummary;
  networkLiveBucket?: DashboardNetworkTimeseriesPoint | null;
  networkRealtimeRate?: DashboardNetworkRealtimeRate | null;
  accounts?: UpstreamAccountActivityAccount[];
}

export interface RoutingStateVersion {
  epoch: string;
  generation: string;
}

export function compareRoutingStateVersion(
  left: RoutingStateVersion | null | undefined,
  right: RoutingStateVersion | null | undefined,
): number {
  if (!left && !right) return 0;
  if (!left) return -1;
  if (!right) return 1;
  const epochOrder = left.epoch.localeCompare(right.epoch);
  if (epochOrder !== 0) return epochOrder;
  try {
    const leftGeneration = BigInt(left.generation);
    const rightGeneration = BigInt(right.generation);
    return leftGeneration === rightGeneration ? 0 : leftGeneration > rightGeneration ? 1 : -1;
  } catch {
    return left.generation.localeCompare(right.generation);
  }
}

export function acceptsRoutingStateVersion(
  current: RoutingStateVersion | null | undefined,
  incoming: RoutingStateVersion | null | undefined,
  kind: "snapshot" | "replay" | "live" | "patch" = "live",
): boolean {
  if (!incoming) return !current || kind === "snapshot" || kind === "patch";
  if (!current) return true;
  const comparison = compareRoutingStateVersion(incoming, current);
  return incoming.epoch !== current.epoch
    ? kind === "snapshot" || kind === "patch"
    : comparison >= 0;
}

export interface DashboardActivityRecentResponse {
  rangeStart: string;
  rangeEnd: string;
  snapshotId: number;
  accounts: Array<{
    accountKey: string;
    recentInvocations: PromptCacheConversationInvocationPreview[];
  }>;
}

export interface DashboardNetworkTimeseriesPoint {
  bucketStart: string;
  bucketEnd: string;
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  uploadBytes: number;
  downloadBytes: number;
  isLiveBucket: boolean;
}

export interface DashboardNetworkRealtimeRate {
  sampleStart: string;
  sampleEnd: string;
  sampleSeconds: number;
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  uploadBytes: number;
  downloadBytes: number;
}

export interface DashboardNetworkTimeseriesResponse {
  range: string;
  rangeStart: string;
  rangeEnd: string;
  snapshotId: number;
  bucketSeconds: number;
  points: DashboardNetworkTimeseriesPoint[];
}

export interface DashboardRecentNetworkWindowPoint {
  sampleStart: string;
  sampleEnd: string;
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  uploadBytes: number;
  downloadBytes: number;
  isAvailable: boolean;
}

export interface DashboardRecentNetworkWindowResponse {
  rangeStart: string;
  rangeEnd: string;
  windowSeconds: number;
  sampleSeconds: number;
  isWarmingUp: boolean;
  points: DashboardRecentNetworkWindowPoint[];
}

export interface StatsMaintenanceResponse {
  rawCompressionBacklog?: RawCompressionBacklogResponse;
  startupBackfill?: StartupBackfillResponse;
  historicalRollupBackfill?: HistoricalRollupBackfillResponse;
}

export interface RawCompressionBacklogResponse {
  oldestUncompressedAgeSecs: number;
  uncompressedCount: number;
  uncompressedBytes: number;
  alertLevel: "ok" | "warn" | "critical";
}

export interface StartupBackfillResponse {
  upstreamActivityArchivePendingAccounts: number;
  zeroUpdateStreak: number;
  nextRunAfter?: string | null;
}

export interface HistoricalRollupBackfillResponse {
  pendingBuckets: number;
  legacyArchivePending: number;
  lastMaterializedHour?: string | null;
  alertLevel: "none" | "warn" | "critical";
}

export interface TimeseriesPoint {
  bucketStart: string;
  bucketEnd: string;
  totalCount: number;
  successCount: number;
  failureCount: number;
  inFlightCount?: number;
  inFlightPhaseCounts?: InvocationPhaseCounts | null;
  totalTokens: number;
  inputTokens?: number;
  outputTokens?: number;
  cacheInputTokens?: number;
  reasoningTokens?: number;
  totalCost: number;
  nonSuccessCost?: number;
  avgTotalMs?: number | null;
  totalLatencySampleCount?: number | null;
  firstByteSampleCount?: number;
  firstByteAvgMs?: number | null;
  firstByteP95Ms?: number | null;
  firstResponseByteTotalSampleCount?: number;
  firstResponseByteTotalAvgMs?: number | null;
  firstResponseByteTotalP95Ms?: number | null;
  firstTokenSampleCount?: number;
  firstTokenAvgMs?: number | null;
  firstTokenP95Ms?: number | null;
}

export interface TimeseriesResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  snapshotId?: number;
  effectiveBucket?: string;
  availableBuckets?: string[];
  bucketLimitedToDaily?: boolean;
  points: TimeseriesPoint[];
}

export interface ParallelWorkPoint {
  bucketStart: string;
  bucketEnd: string;
  parallelCount: number;
}

export interface ParallelWorkConversation {
  conversationId: string;
  start: string;
  end: string;
  requestCount: number;
}

export interface ParallelWorkWindowResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  completeBucketCount: number;
  activeBucketCount: number;
  activeMinuteCount: number | null;
  minCount: number | null;
  maxCount: number | null;
  avgCount: number | null;
  effectiveTimeZone?: string;
  timeZoneFallback?: boolean;
  points: ParallelWorkPoint[];
  conversations?: ParallelWorkConversation[];
}

export interface ParallelWorkStatsResponse {
  current: ParallelWorkWindowResponse;
  minute7d: ParallelWorkWindowResponse;
  hour30d: ParallelWorkWindowResponse;
  dayAll: ParallelWorkWindowResponse;
}

export interface ErrorDistributionItem {
  reason: string;
  count: number;
}

export interface ErrorDistributionResponse {
  rangeStart: string;
  rangeEnd: string;
  items: ErrorDistributionItem[];
}

export type FailureScope = "all" | "service" | "client" | "abort";

export interface FailureSummaryResponse {
  rangeStart: string;
  rangeEnd: string;
  totalFailures: number;
  serviceFailureCount: number;
  clientFailureCount: number;
  clientAbortCount: number;
  actionableFailureCount: number;
  actionableFailureRate: number;
}

export interface PerfStageStats {
  stage: string;
  count: number;
  avgMs: number;
  p50Ms: number;
  p90Ms: number;
  p99Ms: number;
  maxMs: number;
}

export interface PerfStatsResponse {
  rangeStart: string;
  rangeEnd: string;
  items?: PerfStageStats[];
  stages?: PerfStageStats[];
}

export interface PerfStatsQuery {
  range?: string;
  bucket?: string;
  settlementHour?: number;
  timeZone?: string;
  source?: string;
  model?: string;
  endpoint?: string;
}

export interface QuotaSnapshot {
  capturedAt: string;
  amountLimit?: number;
  usedAmount?: number;
  remainingAmount?: number;
  period?: string;
  periodResetTime?: string;
  expireTime?: string;
  isActive: boolean;
  totalCost: number;
  totalRequests: number;
  totalTokens: number;
  lastRequestTime?: string;
  billingType?: string;
  remainingCount?: number;
  usedCount?: number;
  subTypeName?: string;
}

export type BroadcastPayload =
  | {
      type: "records";
      records: ApiInvocation[];
    }
  | {
      type: "dashboardActivityLive";
      snapshot: DashboardActivityLiveSnapshot;
    }
  | {
      type: "pool_attempts";
      invokeId: string;
      attempts: ApiPoolUpstreamRequestAttempt[];
    }
  | {
      type: "summary";
      window: string;
      summary: StatsResponse;
    }
  | {
      type: "quota";
      snapshot: QuotaSnapshot;
    }
  | {
      type: "version";
      version: string;
    };
