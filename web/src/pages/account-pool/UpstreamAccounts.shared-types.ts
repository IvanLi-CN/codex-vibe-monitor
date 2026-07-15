export type RoutingDraft = {
  apiKey: string;
  maskedApiKey: string | null;
  primarySyncIntervalSecs: string;
  secondarySyncIntervalSecs: string;
  priorityAvailableAccountCap: string;
};

export const UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS = 600;

type GroupFilterMode = "all" | "ungrouped" | "search" | "exact";

export type GroupFilterState = {
  mode: GroupFilterMode;
  query: string;
};

export type PersistedUpstreamAccountsFilters = {
  workStatus: string[];
  enableStatus: string[];
  healthStatus: string[];
  tagIds: number[];
  groupFilters: string[];
};

export type UpstreamAccountsLocationState = {
  selectedAccountId?: number;
  openDetail?: boolean;
  openDeleteConfirm?: boolean;
  presetGroupFilter?: GroupFilterState | null;
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

export type AccountBusyActionType = "save" | "sync" | "toggle" | "relogin" | "delete";

export type BusyActionState = {
  routing: boolean;
  accountActions: Set<string>;
};

export const DEFAULT_GROUP_FILTER_STATE: GroupFilterState = {
  mode: "all",
  query: "",
};

export const DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS: PersistedUpstreamAccountsFilters = {
  workStatus: [],
  enableStatus: [],
  healthStatus: [],
  tagIds: [],
  groupFilters: [],
};
