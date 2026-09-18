import type {
  AvailableModelsMode,
  CapabilityOverride,
  CodexImagegenRewriteMode,
  ImageToolRewriteMode,
  RequestCompressionAlgorithm,
  StatusChangeReasonCode,
  TagFastModeRewriteMode,
  TagPriorityTier,
  TagRoutingRule,
} from "./core-upstream-types";
export interface CompleteOauthLoginSessionPayload {
  callbackUrl: string;
  mailboxSessionId?: string;
  mailboxAddress?: string;
}

export interface CreateOauthMailboxSessionPayload {
  emailAddress?: string;
}

export interface OauthMailboxStatusRequestPayload {
  sessionIds: string[];
}

export interface CreateApiKeyAccountPayload {
  displayName: string;
  email?: string;
  note?: string;
  upstreamBaseUrl?: string;
  apiKey: string;
  localPrimaryLimit?: number;
  localSecondaryLimit?: number;
  localLimitUnit?: string;
  tagIds?: number[];
  boundProxyKeys?: string[];
}

export interface ApiKeyGroupMigrationPreflight {
  confirmationHash: string;
  apiKeyCount: number;
  portableFields: string[];
  blockedStrategies: string[];
  canMigrate: boolean;
}

export interface ConfirmApiKeyGroupMigrationPayload {
  confirmationHash: string;
  disabledStrategies: string[];
}

export interface ApiKeyGroupMigrationResult {
  migratedCount: number;
  confirmationHash: string;
  auditAction: string;
}

export interface ApiKeyGroupMigrationPreflight {
  confirmationHash: string;
  apiKeyCount: number;
  portableFields: string[];
  blockedStrategies: string[];
  canMigrate: boolean;
}

export interface ConfirmApiKeyGroupMigrationPayload {
  confirmationHash: string;
  disabledStrategies: string[];
}

export interface ApiKeyGroupMigrationResult {
  migratedCount: number;
  confirmationHash: string;
  auditAction: string;
}

export interface UpdateUpstreamAccountPayload {
  displayName?: string;
  email?: string | null;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  boundProxyKeys?: string[] | null;
  concurrencyLimit?: number;
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  note?: string;
  groupNote?: string;
  upstreamBaseUrl?: string | null;
  enabled?: boolean;
  isMother?: boolean;
  apiKey?: string;
  localPrimaryLimit?: number | null;
  localSecondaryLimit?: number | null;
  localLimitUnit?: string | null;
  tagIds?: number[];
  responseEndpointCapabilityOverride?: CapabilityOverride | null;
  chatCompletionsCapabilityOverride?: CapabilityOverride | null;
  imageEndpointCapabilityOverride?: CapabilityOverride | null;
  responseImageToolCapabilityOverride?: CapabilityOverride | null;
  codexImagegenCapabilityOverride?: CapabilityOverride | null;
  standaloneSearchCapabilityOverride?: CapabilityOverride | null;
  routingRule?: UpdateGroupAccountRoutingRulePayload;
}

export interface ImportOauthCredentialFilePayload {
  sourceId: string;
  fileName: string;
  content: string;
}

export interface ValidateImportedOauthAccountsPayload {
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  items: ImportOauthCredentialFilePayload[];
}

export interface ImportedOauthMatchSummary {
  accountId: number;
  displayName: string;
  groupName?: string | null;
  status: string;
}

export interface ImportedOauthValidationRow {
  sourceId: string;
  fileName: string;
  email?: string | null;
  chatgptAccountId?: string | null;
  chatgptUserId?: string | null;
  displayName?: string | null;
  tokenExpiresAt?: string | null;
  matchedAccount?: ImportedOauthMatchSummary | null;
  status: "pending" | "duplicate_in_input" | "ok" | "ok_exhausted" | "invalid" | "error" | string;
  detail?: string | null;
  attempts: number;
}

export interface ImportedOauthValidationResponse {
  inputFiles: number;
  uniqueInInput: number;
  duplicateInInput: number;
  rows: ImportedOauthValidationRow[];
}

export interface ImportedOauthValidationCounts {
  pending: number;
  duplicateInInput: number;
  ok: number;
  okExhausted: number;
  invalid: number;
  error: number;
  checked: number;
}

export interface ImportedOauthValidationJobResponse {
  jobId: string;
  snapshot: ImportedOauthValidationResponse;
}

export interface ImportedOauthValidationSnapshotEventPayload {
  snapshot: ImportedOauthValidationResponse;
  counts: ImportedOauthValidationCounts;
}

export interface ImportedOauthValidationRowEventPayload {
  row: ImportedOauthValidationRow;
  counts: ImportedOauthValidationCounts;
}

export interface ImportedOauthValidationFailedEventPayload {
  snapshot: ImportedOauthValidationResponse;
  counts: ImportedOauthValidationCounts;
  error: string;
}

export interface ImportValidatedOauthAccountsPayload {
  items: ImportOauthCredentialFilePayload[];
  selectedSourceIds: string[];
  validationJobId?: string;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  groupNote?: string;
  concurrencyLimit?: number;
  tagIds?: number[];
}

export interface ImportedOauthImportResult {
  sourceId: string;
  fileName: string;
  email?: string | null;
  chatgptAccountId?: string | null;
  accountId?: number | null;
  status: "created" | "updated_existing" | "failed" | string;
  detail?: string | null;
  matchedAccount?: ImportedOauthMatchSummary | null;
}

export interface ImportedOauthImportSummary {
  inputFiles: number;
  selectedFiles: number;
  created: number;
  updatedExisting: number;
  failed: number;
}

export interface ImportedOauthImportResponse {
  summary: ImportedOauthImportSummary;
  results: ImportedOauthImportResult[];
}

export interface CreateTagPayload extends TagRoutingRule {
  name: string;
}

export type UpdateTagPayload = Partial<CreateTagPayload>;

export interface FetchTagsQuery {
  search?: string;
  hasAccounts?: boolean;
  allowCutIn?: boolean;
  allowCutOut?: boolean;
}

export interface UpdateUpstreamAccountGroupPayload {
  note?: string;
  boundProxyKeys?: string[];
  concurrencyLimit?: number;
  nodeShuntEnabled?: boolean;
  singleAccountRotationEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  routingRule?: UpdateGroupAccountRoutingRulePayload;
}

export type NullableRoutingRuleValue<T> = T | null;

export interface UpdateGroupAccountRoutingRulePayload {
  allowCutOut?: NullableRoutingRuleValue<boolean>;
  allowCutIn?: NullableRoutingRuleValue<boolean>;
  priorityTier?: NullableRoutingRuleValue<TagPriorityTier>;
  fastModeRewriteMode?: NullableRoutingRuleValue<TagFastModeRewriteMode>;
  imageToolRewriteMode?: NullableRoutingRuleValue<ImageToolRewriteMode>;
  codexImagegenRewriteMode?: NullableRoutingRuleValue<CodexImagegenRewriteMode>;
  requestCompressionAlgorithm?: NullableRoutingRuleValue<RequestCompressionAlgorithm>;
  concurrencyLimit?: NullableRoutingRuleValue<number>;
  upstream429RetryEnabled?: NullableRoutingRuleValue<boolean>;
  upstream429MaxRetries?: NullableRoutingRuleValue<number>;
  availableModels?: NullableRoutingRuleValue<string[]>;
  availableModelsMode?: NullableRoutingRuleValue<AvailableModelsMode>;
  statusChangeReasons?: Partial<Record<StatusChangeReasonCode, NullableRoutingRuleValue<boolean>>>;
  timeouts?: {
    responsesFirstByteTimeoutSecs?: NullableRoutingRuleValue<number>;
    compactFirstByteTimeoutSecs?: NullableRoutingRuleValue<number>;
    imageFirstByteTimeoutSecs?: NullableRoutingRuleValue<number>;
    responsesStreamTimeoutSecs?: NullableRoutingRuleValue<number>;
    compactStreamTimeoutSecs?: NullableRoutingRuleValue<number>;
  };
}
