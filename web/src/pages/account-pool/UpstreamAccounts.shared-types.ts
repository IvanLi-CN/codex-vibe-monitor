import type { PoolRoutingTimeoutSettings } from "../../lib/api";

export type RoutingDraft = {
  apiKey: string;
  maskedApiKey: string | null;
  primarySyncIntervalSecs: string;
  secondarySyncIntervalSecs: string;
  priorityAvailableAccountCap: string;
  responsesFirstByteTimeoutSecs: string;
  compactFirstByteTimeoutSecs: string;
  responsesStreamTimeoutSecs: string;
  compactStreamTimeoutSecs: string;
};

export const DEFAULT_ROUTING_TIMEOUTS: PoolRoutingTimeoutSettings = {
  responsesFirstByteTimeoutSecs: 120,
  compactFirstByteTimeoutSecs: 300,
  responsesStreamTimeoutSecs: 300,
  compactStreamTimeoutSecs: 300,
};

export const UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS = 600;

type GroupFilterMode = "all" | "ungrouped" | "search";

export type GroupFilterState = {
  mode: GroupFilterMode;
  query: string;
};

export type PersistedUpstreamAccountsFilters = {
  workStatus: string[];
  enableStatus: string[];
  healthStatus: string[];
  tagIds: number[];
  groupFilter: GroupFilterState;
};

export type UpstreamAccountsLocationState = {
  selectedAccountId?: number;
  openDetail?: boolean;
  openDeleteConfirm?: boolean;
  postCreateWarning?: string | null;
  duplicateWarning?: {
    accountId: number;
    displayName: string;
    peerAccountIds: number[];
    reasons: string[];
  } | null;
};

export type ActionErrorState = {
  routing: string | null;
  accountMessages: Record<number, string>;
};

export type AccountBusyActionType =
  | "save"
  | "sync"
  | "toggle"
  | "relogin"
  | "delete";

export type BusyActionState = {
  routing: boolean;
  accountActions: Set<string>;
};

export const DEFAULT_GROUP_FILTER_STATE: GroupFilterState = {
  mode: "all",
  query: "",
};

export const DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS: PersistedUpstreamAccountsFilters =
  {
    workStatus: [],
    enableStatus: [],
    healthStatus: [],
    tagIds: [],
    groupFilter: DEFAULT_GROUP_FILTER_STATE,
  };
