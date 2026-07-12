import {
  buildDefaultStatusChangeReasonFieldSources,
  buildDefaultStatusChangeReasons,
  type EffectiveRoutingRuleSource,
  STATUS_CHANGE_REASON_CODES,
  type StatusChangeReasonCode,
  type StatusChangeReasonFieldSources,
  type StatusChangeReasons,
} from "./api/core-upstream";

export type { StatusChangeReasonCode, StatusChangeReasonFieldSources, StatusChangeReasons };
export {
  buildDefaultStatusChangeReasonFieldSources,
  buildDefaultStatusChangeReasons,
  STATUS_CHANGE_REASON_CODES,
};

export type StatusChangeReasonGroupId = "auth" | "quota" | "availability";

export type StatusChangeReasonFieldKey = `statusChangeReason:${StatusChangeReasonCode}`;

export const STATUS_CHANGE_REASON_GROUPS: Array<{
  id: StatusChangeReasonGroupId;
  reasonCodes: StatusChangeReasonCode[];
}> = [
  {
    id: "auth",
    reasonCodes: ["upstream_http_401", "upstream_http_402", "upstream_http_403", "reauth_required"],
  },
  {
    id: "quota",
    reasonCodes: [
      "upstream_http_429_rate_limit",
      "upstream_http_429_quota_exhausted",
      "usage_snapshot_exhausted",
      "quota_still_exhausted",
    ],
  },
  {
    id: "availability",
    reasonCodes: ["transport_failure", "upstream_server_overloaded", "upstream_http_5xx"],
  },
];

const STATUS_CHANGE_REASON_CODE_SET = new Set<string>(STATUS_CHANGE_REASON_CODES);

export function resolveStatusChangeReasons(
  value?: Partial<StatusChangeReasons> | null,
): StatusChangeReasons {
  const next = buildDefaultStatusChangeReasons();
  if (!value) return next;
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (typeof value[reason] === "boolean") {
      next[reason] = value[reason] === true;
    }
  }
  return next;
}

export function resolveStatusChangeReasonFieldSources(
  value?: Partial<StatusChangeReasonFieldSources> | null,
  fallback: EffectiveRoutingRuleSource = "root",
): StatusChangeReasonFieldSources {
  const next = buildDefaultStatusChangeReasonFieldSources(fallback);
  if (!value) return next;
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (typeof value[reason] === "string" && value[reason].trim()) {
      next[reason] = value[reason] as EffectiveRoutingRuleSource;
    }
  }
  return next;
}

export function statusChangeReasonFieldKey(
  reason: StatusChangeReasonCode,
): StatusChangeReasonFieldKey {
  return `statusChangeReason:${reason}`;
}

export function statusChangeReasonFromFieldKey(field: string): StatusChangeReasonCode | null {
  const prefix = "statusChangeReason:";
  if (!field.startsWith(prefix)) return null;
  const reason = field.slice(prefix.length);
  return STATUS_CHANGE_REASON_CODE_SET.has(reason) ? (reason as StatusChangeReasonCode) : null;
}

export function countEnabledStatusChangeReasons(
  settings: StatusChangeReasons,
  reasonCodes: readonly StatusChangeReasonCode[] = STATUS_CHANGE_REASON_CODES,
): number {
  return reasonCodes.filter((reason) => settings[reason] !== false).length;
}

export function setStatusChangeReasonGroupValue(
  settings: StatusChangeReasons,
  reasonCodes: readonly StatusChangeReasonCode[],
  enabled: boolean,
): StatusChangeReasons {
  const next = { ...settings };
  for (const reason of reasonCodes) {
    next[reason] = enabled;
  }
  return next;
}
