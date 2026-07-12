import type { StatusChangeReasonCode } from "../../lib/upstreamAccountStatusChangeReasons";
import type { AppIconName } from "../shared/AppIcon";

const statusChangeReasonIcons = {
  upstream_http_401: "key-outline",
  upstream_http_402: "currency-usd",
  upstream_http_403: "shield-key-outline",
  reauth_required: "login-variant",
  upstream_http_429_rate_limit: "speedometer",
  upstream_http_429_quota_exhausted: "counter",
  usage_snapshot_exhausted: "database-outline",
  quota_still_exhausted: "timer-refresh-outline",
  transport_failure: "server-network-outline",
  upstream_server_overloaded: "lightning-bolt",
  upstream_http_5xx: "alert-decagram-outline",
} satisfies Record<StatusChangeReasonCode, AppIconName>;

export function statusChangeReasonIconName(reason: StatusChangeReasonCode): AppIconName {
  return statusChangeReasonIcons[reason];
}
