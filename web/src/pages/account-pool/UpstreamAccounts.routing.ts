import type { PoolRoutingMaintenanceSettings } from "../../lib/api";
import { DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS } from "../../lib/api";
import type { RoutingDraft } from "./UpstreamAccounts.shared-types";

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
  } | null,
): RoutingDraft {
  const maintenance = resolveRoutingMaintenance(routing?.maintenance);
  return {
    apiKey: "",
    maskedApiKey: routing?.maskedApiKey ?? null,
    primarySyncIntervalSecs: String(maintenance.primarySyncIntervalSecs),
    secondarySyncIntervalSecs: String(maintenance.secondarySyncIntervalSecs),
    priorityAvailableAccountCap: String(maintenance.priorityAvailableAccountCap),
  };
}

export function parseRoutingPositiveInteger(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed || !/^\d+$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  return Number.isSafeInteger(parsed) ? parsed : null;
}
