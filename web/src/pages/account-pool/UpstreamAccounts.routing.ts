import type {
  PoolRoutingMaintenanceSettings,
  PoolRoutingTimeoutSettings,
} from "../../lib/api";
import { DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS } from "../../lib/api";
import {
  DEFAULT_ROUTING_TIMEOUTS,
  type RoutingDraft,
} from "./UpstreamAccounts.shared-types";

const POSITIVE_INTEGER_PATTERN = /^[1-9]\d*$/;

export function resolveRoutingMaintenance(
  maintenance?: PoolRoutingMaintenanceSettings | null,
): PoolRoutingMaintenanceSettings {
  return {
    primarySyncIntervalSecs:
      maintenance?.primarySyncIntervalSecs ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.primarySyncIntervalSecs,
    secondarySyncIntervalSecs:
      maintenance?.secondarySyncIntervalSecs ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.secondarySyncIntervalSecs,
    priorityAvailableAccountCap:
      maintenance?.priorityAvailableAccountCap ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.priorityAvailableAccountCap,
  };
}

export function buildRoutingDraft(
  routing?: {
    maskedApiKey?: string | null;
    maintenance?: PoolRoutingMaintenanceSettings | null;
    timeouts?: PoolRoutingTimeoutSettings | null;
  } | null,
): RoutingDraft {
  const maintenance = resolveRoutingMaintenance(routing?.maintenance);
  const timeouts = routing?.timeouts ?? DEFAULT_ROUTING_TIMEOUTS;
  return {
    apiKey: "",
    maskedApiKey: routing?.maskedApiKey ?? null,
    primarySyncIntervalSecs: String(maintenance.primarySyncIntervalSecs),
    secondarySyncIntervalSecs: String(maintenance.secondarySyncIntervalSecs),
    priorityAvailableAccountCap: String(
      maintenance.priorityAvailableAccountCap,
    ),
    responsesFirstByteTimeoutSecs: String(
      timeouts.responsesFirstByteTimeoutSecs,
    ),
    compactFirstByteTimeoutSecs: String(timeouts.compactFirstByteTimeoutSecs),
    responsesStreamTimeoutSecs: String(timeouts.responsesStreamTimeoutSecs),
    compactStreamTimeoutSecs: String(timeouts.compactStreamTimeoutSecs),
  };
}

export function parseRoutingPositiveInteger(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed || !/^\d+$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

export function parseRoutingTimeoutValue(
  raw: string,
  label: string,
): { ok: true; value: number } | { ok: false; error: string } {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: false, error: `${label} is required.` };
  }
  if (!POSITIVE_INTEGER_PATTERN.test(trimmed)) {
    return { ok: false, error: `${label} must be a positive integer.` };
  }
  const parsed = Number(trimmed);
  if (!Number.isSafeInteger(parsed)) {
    return { ok: false, error: `${label} must be a positive integer.` };
  }
  return { ok: true, value: parsed };
}
