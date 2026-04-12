import type { AppIconName } from "../../components/AppIcon";
import type {
  CompactSupportState,
  UpstreamAccountSummary,
} from "../../lib/api";
import type { TranslationValues } from "../../i18n";
import type {
  AccountBusyActionType,
  BusyActionState,
} from "./UpstreamAccounts.shared-types";

type AccountStatusSnapshot = Pick<
  UpstreamAccountSummary,
  | "status"
  | "displayStatus"
  | "enabled"
  | "workStatus"
  | "enableStatus"
  | "healthStatus"
  | "syncState"
>;

function createBusyActionKey(type: AccountBusyActionType, accountId: number) {
  return `${type}:${accountId}`;
}

function formatDateTime(value?: string | null): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function accountEnableStatus(item?: AccountStatusSnapshot | null) {
  if (item?.enableStatus) return item.enableStatus;
  if (item?.enabled === false || item?.displayStatus === "disabled") {
    return "disabled";
  }
  return "enabled";
}

function accountSyncState(item?: AccountStatusSnapshot | null) {
  if (item?.syncState) return item.syncState;
  const legacyStatus = item?.displayStatus ?? item?.status;
  return legacyStatus === "syncing" ? "syncing" : "idle";
}

export function isBusyAction(
  busyAction: BusyActionState,
  type: AccountBusyActionType | "routing",
  accountId?: number,
): boolean {
  if (type === "routing") return busyAction.routing;
  if (typeof accountId !== "number") return false;
  return busyAction.accountActions.has(createBusyActionKey(type, accountId));
}

export function accountWorkStatus(item?: AccountStatusSnapshot | null) {
  if (!item) return "idle";
  if (accountEnableStatus(item) !== "enabled") return "idle";
  if (accountSyncState(item) === "syncing") return "idle";
  if (item?.workStatus === "degraded") return "degraded";
  if (item?.workStatus === "rate_limited") return "rate_limited";
  if (accountHealthStatus(item) !== "normal") return "unavailable";
  return item?.workStatus ?? "idle";
}

export function accountHealthStatus(item?: AccountStatusSnapshot | null) {
  if (item?.healthStatus) return item.healthStatus;
  const legacyStatus = item?.displayStatus ?? item?.status ?? "error_other";
  if (
    legacyStatus === "needs_reauth" ||
    legacyStatus === "upstream_unavailable" ||
    legacyStatus === "upstream_rejected" ||
    legacyStatus === "error_other"
  ) {
    return legacyStatus;
  }
  if (legacyStatus === "error") {
    return "error_other";
  }
  return "normal";
}

export function compactSupportLabel(
  support: CompactSupportState | null | undefined,
  t: (key: string) => string,
) {
  if (!support || support.status !== "unsupported") return null;
  return t("accountPool.upstreamAccounts.compactSupport.unsupportedBadge");
}

export function compactSupportHint(
  support: CompactSupportState | null | undefined,
  t: (key: string, values?: TranslationValues) => string,
) {
  if (!support || support.status === "unknown") return null;
  const statusLabel =
    support.status === "unsupported"
      ? t("accountPool.upstreamAccounts.compactSupport.status.unsupported")
      : t("accountPool.upstreamAccounts.compactSupport.status.supported");
  const observedAt = support.observedAt
    ? formatDateTime(support.observedAt)
    : t("accountPool.upstreamAccounts.unavailable");
  if (support.reason) {
    return `${statusLabel} · ${observedAt} · ${support.reason}`;
  }
  return `${statusLabel} · ${observedAt}`;
}

export function poolCardMetric(
  value: number | string,
  label: string,
  icon: AppIconName,
  accent: string,
) {
  return { value, label, icon, accent };
}
