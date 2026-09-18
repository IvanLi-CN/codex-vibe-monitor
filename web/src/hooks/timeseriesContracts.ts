import type { ApiInvocation, TimeseriesResponse } from "../lib/api";

export interface UseTimeseriesOptions {
  bucket?: string;
  settlementHour?: number;
  preferServerAggregation?: boolean;
  upstreamAccountId?: number;
}

export interface LiveRecordDelta {
  recordId?: number;
  bucketStart: string;
  bucketEnd: string;
  bucketStartEpoch: number;
  bucketEndEpoch: number;
  totalCount: number;
  successCount: number;
  failureCount: number;
  inFlightCount: number;
  totalTokens: number;
  totalCost: number;
  totalLatencyMs: number;
  totalLatencySampleCount: number;
  countsOnly?: boolean;
}

export interface WriteTimeseriesRemountCacheOptions {
  range: string;
  options?: UseTimeseriesOptions;
  data: TimeseriesResponse;
  cachedAt?: number;
  liveRecordDeltas?: ReadonlyMap<string, LiveRecordDelta> | null;
  settledLiveRecordUpdatedAt?: ReadonlyMap<string, number> | null;
  untrackedInFlightCounts?: ReadonlyMap<string, number> | null;
  untrackedInFlightClaimSnapshotId?: number | null;
}

export interface TrackTimeseriesLiveRecordDeltaOptions {
  liveRecordDeltas: Map<string, LiveRecordDelta>;
  settledLiveRecordUpdatedAt: Map<string, number>;
  key: string;
  record: ApiInvocation;
  delta: LiveRecordDelta | null;
  now?: number;
  ttlMs?: number;
  maxEntries?: number;
}
