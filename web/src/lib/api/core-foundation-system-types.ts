import type {
  ForwardProxySettings,
  PricingSettings,
  ProxySettings,
} from "./core-foundation-settings-types";
export interface SettingsPayload {
  proxy: ProxySettings;
  forwardProxy: ForwardProxySettings;
  pricing: PricingSettings;
}

export interface SystemStatusMetric {
  count: number;
  bytes: number;
}

export interface SystemProjectionConsumerHealth {
  state: string;
  cursorLag: number;
  dirtyBucketCount: number;
  pendingEventCount: number;
  lastFlushElapsedMs?: number;
  lastFlushAgeMs?: number;
  lastRepairScope?: string;
  lastDeferReason?: string;
  lastErrorKind?: string;
}

export interface SystemProjectionHealth {
  terminal: SystemProjectionConsumerHealth;
  longTerm: SystemProjectionConsumerHealth;
}

export interface SystemRawMetricsHealth {
  state: string;
  inventoryCursor: number;
  updatedAgeMs?: number;
}

export interface RuntimePressureProcessHealth {
  rssBytes: number;
  rssAnonBytes: number;
  swapBytes: number;
  peakRssBytes: number;
  threads: number;
  managedBytes: number;
  unattributedAnonBytes: number;
  pressureLevel: string;
}

export interface RuntimePressureWriterAccountingHealth {
  state: string;
  pendingDepth: number;
  pendingBytes: number;
  transferBytes: number;
  retryCount: number;
  p2FlushAttemptCount?: number;
  p2PressureDeferCount?: number;
  p2LockRetryCount?: number;
  p2NextAttemptInMs?: number;
  p2DeferredAgeMs?: number;
  p2WakeReason?: string;
  invariantViolationCount: number;
  degradedReason?: string;
}

export interface RuntimePressurePromptCacheProjectionHealth {
  mode: string;
  activeTopicCount: number;
  dirtyKeyCount: number;
  coalescedEventCount: number;
  fullHydrationCount: number;
  boundedKeyHydrationCount: number;
  livePathDbReadCount: number;
  baselineAgeMs: number;
  responseSource: string;
}

export interface RuntimePressureProxySqliteWriteCoordinatorHealth {
  mode: string;
  activeWriteClass?: string;
  p1WaiterCount: number;
  interactiveWaiterCount: number;
  p2WaiterCount: number;
  maintenanceWaiterCount: number;
  maintenanceFairnessAdmissionCount: number;
  directWriteBypassCount: number;
}

export interface RuntimePressureRetentionWriteHealth {
  state: string;
  operation?: string;
  admissionMode?: string;
  batchRows: number;
  estimatedBytes: number;
  prepareElapsedMs: number;
  lockWaitMs: number;
  executeMs: number;
  commitMs: number;
  budgetBreachCount: number;
  deferReason?: string;
  starvationAgeMs?: number;
  p1WaiterCount: number;
  candidateRemainingHint: number;
  lastError?: string;
}

export interface RuntimePressureDashboardProjectionHealth {
  mode: string;
  state: string;
  producerState: string;
  activeSubscriberCount: number;
  livePathDbReadCount: number;
  buildCount: number;
  revision: number;
  snapshotOrigin: string;
  lastGoodAgeMs?: number;
  degradedReason?: string;
  lastDeferReason?: string;
  sliceCounters: RuntimePressureProjectionSliceCounters;
}

export interface RuntimePressureProjectionSliceHealth {
  buildCount: number;
  revisionCount: number;
  cadenceMissCount: number;
}

export interface RuntimePressureProjectionSliceCounters {
  current: RuntimePressureProjectionSliceHealth;
  network: RuntimePressureProjectionSliceHealth;
  terminal: RuntimePressureProjectionSliceHealth;
}

export interface RuntimePressureDeliveryTopicHealth {
  materializationCount: number;
  serializationCount: number;
  payloadCloneCount: number;
  frameBytesCount: number;
  laggedCount: number;
  skippedCount: number;
  businessPayloadCount: number;
  jsonOverlayCount: number;
}

export interface RuntimePressureDeliveryHealth {
  activity: RuntimePressureDeliveryTopicHealth;
  summary: RuntimePressureDeliveryTopicHealth;
  networkTimeseries: RuntimePressureDeliveryTopicHealth;
  networkRecent: RuntimePressureDeliveryTopicHealth;
}

export interface RuntimePressureDashboardHotTopicHealth {
  topicClass: string;
  state: string;
  activeSubscriberCount: number;
  builderCount: number;
  genericFallbackBuildCount: number;
  livePathDbReadCount: number;
  materializationCount: number;
  serializationCount: number;
  payloadCloneCount: number;
  frameReused: number;
  cadenceMissCount: number;
  reconnectChurnCount: number;
}

export interface RuntimePressureDashboardHotTopicsHealth {
  state: string;
  activity: RuntimePressureDashboardHotTopicHealth;
  summary: RuntimePressureDashboardHotTopicHealth;
  networkTimeseries: RuntimePressureDashboardHotTopicHealth;
  networkRecent: RuntimePressureDashboardHotTopicHealth;
  workingConversations: RuntimePressureDashboardHotTopicHealth;
  parallelWork: RuntimePressureDashboardHotTopicHealth;
  timeseries: RuntimePressureDashboardHotTopicHealth;
}

export interface RuntimePressureRequestPipelineHealth {
  mode: string;
  lastSnapshotKind: string;
  semanticParseCount: number;
  wholeBodyMaterializationCount: number;
  rewriteBufferPeakBytes: number;
  lastFallbackReason?: string;
  parseWindowCount?: number;
  parseWindowCpuMs?: number;
  parseWindowWallMs?: number;
  parseWindowBytes?: number;
}

export interface RuntimePressureEventBusHealth {
  state: string;
  publishedCount: number;
  processedEventCount: number;
  coalescedEventCount: number;
  businessPayloadCloneCount: number;
  topicWorkCount: number;
  routerLaggedCount: number;
  routerGapCount: number;
  cursorRecoveryCount: number;
}

export interface RuntimePressureBackfillHealth {
  state: string;
  wakeGeneration: number;
  wakeCount: number;
  dueDispatchCount: number;
  noopSuppressedCount: number;
  pressureDeferCount: number;
  failureCount: number;
  wokenTaskCount: number;
  scheduledTaskCount: number;
  deferredTaskCount: number;
  failedTaskCount: number;
}

export interface RuntimePressureHealth {
  state: string;
  process: RuntimePressureProcessHealth;
  allocator: { mallocArenaMax: string };
  writerAccounting: RuntimePressureWriterAccountingHealth;
  promptCacheProjection?: RuntimePressurePromptCacheProjectionHealth;
  proxySqliteWriteCoordinator?: RuntimePressureProxySqliteWriteCoordinatorHealth;
  retentionWriteHealth?: RuntimePressureRetentionWriteHealth;
  dashboardProjection: RuntimePressureDashboardProjectionHealth;
  delivery: RuntimePressureDeliveryHealth;
  dashboardHotTopics?: RuntimePressureDashboardHotTopicsHealth;
  requestPipeline?: RuntimePressureRequestPipelineHealth;
  eventBus?: RuntimePressureEventBusHealth;
  backfill?: RuntimePressureBackfillHealth;
}

export interface SystemStatusResponse {
  liveInvocationsCount: number;
  successCount: number;
  nonSuccessCount: number;
  completedArchiveBatchesCount: number;
  archivedBodies: SystemStatusMetric;
  rawBodies: SystemStatusMetric;
  requestRawBodies: SystemStatusMetric;
  responseRawBodies: SystemStatusMetric;
  databaseBytes: number;
  otherFilesBytes: number;
  projectionHealth: SystemProjectionHealth;
  rawMetricsHealth: SystemRawMetricsHealth;
  runtimePressureHealth?: RuntimePressureHealth;
  refreshedAt: string;
}

export interface SystemTaskRun {
  id: number;
  taskKind: string;
  triggerKind: string;
  status: string;
  summary?: string;
  detail?: string;
  startedAt: string;
  finishedAt?: string;
  durationMs?: number;
}

export interface SystemTaskRunsResponse {
  items: SystemTaskRun[];
  total: number;
  page: number;
  pageSize: number;
  nextCursor?: string;
}

export interface ExternalApiKeySummary {
  id: number;
  name: string;
  status: string;
  prefix: string;
  lastUsedAt?: string;
  createdAt: string;
  updatedAt: string;
}

export interface ExternalApiKeyListResponse {
  items: ExternalApiKeySummary[];
}

export interface ExternalApiKeyMutationResponse {
  key: ExternalApiKeySummary;
}

export interface ExternalApiKeySecretResponse {
  key: ExternalApiKeySummary;
  secret: string;
}
