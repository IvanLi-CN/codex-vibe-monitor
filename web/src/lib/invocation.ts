import type { TranslationKey } from "../i18n";
import type { ApiInvocation } from "./api";

const DEFAULT_FALLBACK = "—";
const PRIORITY_SERVICE_TIER = "priority";
const ROUTE_MODE_POOL = "pool";
const RESPONSES_ENDPOINT = "/v1/responses";
const CHAT_COMPLETIONS_ENDPOINT = "/v1/chat/completions";
const COMPACT_ENDPOINT = "/v1/responses/compact";
const IMAGE_ENDPOINT_PREFIX = "/v1/images/";
const IMAGE_GENERATIONS_ENDPOINT = "/v1/images/generations";
const IMAGE_EDITS_ENDPOINT = "/v1/images/edits";
const RUNNING_STATUSES = new Set(["running", "pending"]);

export type ProxyWeightDeltaDirection = "up" | "down" | "flat" | "missing";
export type FastIndicatorState = "effective" | "requested_only" | "none";
export type InvocationEndpointKind =
  | "responses"
  | "chat"
  | "compact"
  | "remote_v2"
  | "image_gen"
  | "image_edit"
  | "image"
  | "raw";
export type InvocationCompactionKind = "compact" | "remote_v2";
export type InvocationImageIntent = "yes" | "direct_image" | "no" | "unknown";
export type InvocationImageBadgeVariant = "success" | "info";
type InvocationImageBadgeLabelKey = "table.imageTool.badge";

type InvocationEndpointBadgeVariant = "default" | "secondary" | "info";
type InvocationEndpointBadgeLabelKey =
  | "table.endpoint.responsesBadge"
  | "table.endpoint.chatBadge"
  | "table.endpoint.compactBadge"
  | "table.endpoint.remoteV2Badge"
  | "table.endpoint.imageGenBadge"
  | "table.endpoint.imageEditBadge"
  | "table.endpoint.imageBadge";

export interface ProxyWeightDeltaView {
  direction: ProxyWeightDeltaDirection;
  value: string;
}

export interface InvocationEndpointDisplay {
  kind: InvocationEndpointKind;
  endpointValue: string;
  badgeVariant: InvocationEndpointBadgeVariant | null;
  labelKey: InvocationEndpointBadgeLabelKey | null;
}

export interface InvocationImageIntentDisplay {
  kind: InvocationImageIntent | "missing";
  showsBadge: boolean;
  badgeVariant: InvocationImageBadgeVariant | null;
  badgeLabelKey: InvocationImageBadgeLabelKey | null;
  detailLabelKey: TranslationKey | null;
}

export interface InvocationModelDisplay {
  primaryValue: string;
  requestValue: string | null;
  responseValue: string | null;
  hasMismatch: boolean;
}

export function isImageInvocationEndpointKind(kind: InvocationEndpointKind) {
  return kind === "image_gen" || kind === "image_edit" || kind === "image";
}

function normalizeImageIntent(value: string | null | undefined): InvocationImageIntent | null {
  if (value === "yes" || value === "direct_image" || value === "no" || value === "unknown") {
    return value;
  }
  return null;
}

function normalizeCompactionKind(
  value: string | null | undefined,
): InvocationCompactionKind | null {
  if (value === "compact" || value === "remote_v2") return value;
  return null;
}

function normalizeInvocationStatus(value: string | null | undefined) {
  if (typeof value !== "string") return "";
  return value.trim().toLowerCase();
}

function normalizeInvocationTimingStage(value: number | null | undefined): number | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return null;
  }
  return value;
}

function normalizeModelValue(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
}

function datedModelAliasBase(model: string): string | null {
  const match = model.match(/^(.*)-\d{4}-\d{2}-\d{2}$/);
  if (!match) return null;
  const base = match[1]?.trim();
  return base ? base : null;
}

export function normalizeModelComparisonKey(value: string | null | undefined): string | null {
  const normalized = normalizeModelValue(value);
  if (!normalized) return null;
  const aliasBase = datedModelAliasBase(normalized) ?? normalized;
  return aliasBase.toLowerCase();
}

export function areInvocationModelsEquivalent(
  left: string | null | undefined,
  right: string | null | undefined,
): boolean {
  const leftKey = normalizeModelComparisonKey(left);
  const rightKey = normalizeModelComparisonKey(right);
  if (leftKey == null || rightKey == null) return false;
  return leftKey === rightKey;
}

export function resolveInvocationModelDisplay(
  record: Pick<ApiInvocation, "model" | "requestModel" | "responseModel">,
): InvocationModelDisplay {
  const requestValue = normalizeModelValue(record.requestModel);
  const responseValue = normalizeModelValue(record.responseModel);
  const legacyValue = normalizeModelValue(record.model);
  const primaryValue = responseValue ?? legacyValue ?? requestValue ?? DEFAULT_FALLBACK;
  const hasMismatch =
    requestValue != null &&
    responseValue != null &&
    !areInvocationModelsEquivalent(requestValue, responseValue);

  return {
    primaryValue,
    requestValue,
    responseValue,
    hasMismatch,
  };
}

export function normalizeServiceTier(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase();
  return normalized.length > 0 ? normalized : null;
}

export function formatServiceTier(
  value: string | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): string {
  return normalizeServiceTier(value) ?? fallback;
}

export function isPriorityServiceTier(value: string | null | undefined): boolean {
  return normalizeServiceTier(value) === PRIORITY_SERVICE_TIER;
}

export function getFastIndicatorState(
  requestedServiceTier: string | null | undefined,
  _effectiveServiceTier: string | null | undefined,
  billingServiceTier?: string | null | undefined,
): FastIndicatorState {
  if (isPriorityServiceTier(billingServiceTier)) return "effective";
  if (isPriorityServiceTier(requestedServiceTier)) return "requested_only";
  return "none";
}

export function formatProxyWeightDelta(
  value: number | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): ProxyWeightDeltaView {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return { direction: "missing", value: fallback };
  }
  const normalized = Object.is(value, -0) ? 0 : value;
  const rounded = Number(normalized.toFixed(2));
  if (rounded > 0) return { direction: "up", value: Math.abs(rounded).toFixed(2) };
  if (rounded < 0) return { direction: "down", value: Math.abs(rounded).toFixed(2) };
  return { direction: "flat", value: Math.abs(rounded).toFixed(2) };
}

export function normalizeRouteMode(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const normalized = value.trim().toLowerCase();
  return normalized.length > 0 ? normalized : null;
}

export function isPoolRouteMode(value: string | null | undefined): boolean {
  return normalizeRouteMode(value) === ROUTE_MODE_POOL;
}

export function isInvocationPoolAccountRoutingInProgress(
  routeMode: string | null | undefined,
  status: string | null | undefined,
  upstreamAccountName: string | null | undefined,
  upstreamAccountId: number | null | undefined,
): boolean {
  if (!isPoolRouteMode(routeMode)) return false;
  const normalizedStatus = status?.trim().toLowerCase();
  if (normalizedStatus !== "running" && normalizedStatus !== "pending") {
    return false;
  }
  const name = upstreamAccountName?.trim();
  if (name) return true;
  return typeof upstreamAccountId === "number" && Number.isFinite(upstreamAccountId);
}

export function resolveInvocationAccountLabel(
  routeMode: string | null | undefined,
  status: string | null | undefined,
  failureKind: string | null | undefined,
  errorMessage: string | null | undefined,
  upstreamAccountName: string | null | undefined,
  upstreamAccountId: number | null | undefined,
  reverseProxyLabel: string,
  poolRoutingPendingLabel: string,
  poolAccountUnknownLabel: string,
  poolAccountUnavailableLabel: string,
): string {
  if (!isPoolRouteMode(routeMode)) return reverseProxyLabel;

  const name = upstreamAccountName?.trim();
  if (name) return name;
  if (typeof upstreamAccountId === "number" && Number.isFinite(upstreamAccountId)) {
    return `账号 #${Math.trunc(upstreamAccountId)}`;
  }
  const normalizedStatus = status?.trim().toLowerCase();
  if (normalizedStatus === "running" || normalizedStatus === "pending") {
    return poolRoutingPendingLabel;
  }
  const normalizedFailureKind = failureKind?.trim().toLowerCase();
  const normalizedErrorMessage = errorMessage?.trim().toLowerCase() ?? "";
  if (
    normalizedFailureKind === "pool_no_available_account" ||
    (normalizedFailureKind === "pool_routing_blocked" &&
      !normalizedErrorMessage.includes("sticky conversation cannot cut out of the current account"))
  ) {
    return poolAccountUnavailableLabel;
  }
  return poolAccountUnknownLabel;
}

export function formatResponseContentEncoding(
  value: string | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): string {
  if (typeof value !== "string") return fallback;
  const normalized = value.trim().toLowerCase();
  return normalized.length > 0 ? normalized : fallback;
}

export function resolveInvocationEndpointDisplay(
  record:
    | Pick<
        ApiInvocation,
        "endpoint" | "status" | "compactionRequestKind" | "compactionResponseKind"
      >
    | string
    | null
    | undefined,
  fallback: string = DEFAULT_FALLBACK,
): InvocationEndpointDisplay {
  const endpointValue =
    typeof record === "string"
      ? record.trim()
      : typeof record?.endpoint === "string"
        ? record.endpoint.trim()
        : "";
  const status =
    typeof record === "string" || record == null ? "" : normalizeInvocationStatus(record.status);
  const compactionRequestKind =
    typeof record === "string" || record == null
      ? null
      : normalizeCompactionKind(record.compactionRequestKind);
  const compactionResponseKind =
    typeof record === "string" || record == null
      ? null
      : normalizeCompactionKind(record.compactionResponseKind);

  if (endpointValue === IMAGE_GENERATIONS_ENDPOINT) {
    return {
      kind: "image_gen",
      endpointValue,
      badgeVariant: "info",
      labelKey: "table.endpoint.imageGenBadge",
    };
  }

  if (endpointValue === IMAGE_EDITS_ENDPOINT) {
    return {
      kind: "image_edit",
      endpointValue,
      badgeVariant: "secondary",
      labelKey: "table.endpoint.imageEditBadge",
    };
  }

  if (endpointValue.startsWith(IMAGE_ENDPOINT_PREFIX)) {
    return {
      kind: "image",
      endpointValue,
      badgeVariant: "secondary",
      labelKey: "table.endpoint.imageBadge",
    };
  }

  switch (endpointValue) {
    case RESPONSES_ENDPOINT:
      if (
        compactionResponseKind === "remote_v2" ||
        (RUNNING_STATUSES.has(status) && compactionRequestKind === "remote_v2")
      ) {
        return {
          kind: "remote_v2",
          endpointValue,
          badgeVariant: "info",
          labelKey: "table.endpoint.remoteV2Badge",
        };
      }
      return {
        kind: "responses",
        endpointValue,
        badgeVariant: "default",
        labelKey: "table.endpoint.responsesBadge",
      };
    case CHAT_COMPLETIONS_ENDPOINT:
      return {
        kind: "chat",
        endpointValue,
        badgeVariant: "secondary",
        labelKey: "table.endpoint.chatBadge",
      };
    case COMPACT_ENDPOINT:
      return {
        kind: "compact",
        endpointValue,
        badgeVariant: "info",
        labelKey: "table.endpoint.compactBadge",
      };
    default:
      return {
        kind: "raw",
        endpointValue: endpointValue || fallback,
        badgeVariant: null,
        labelKey: null,
      };
  }
}

export function resolveInvocationImageIntentDisplay(
  record: Pick<ApiInvocation, "imageIntent"> | ApiInvocation["imageIntent"] | null | undefined,
): InvocationImageIntentDisplay {
  const imageIntent =
    typeof record === "string" || record == null
      ? normalizeImageIntent(record)
      : normalizeImageIntent(record.imageIntent);

  switch (imageIntent) {
    case "yes":
      return {
        kind: "yes",
        showsBadge: true,
        badgeVariant: "success",
        badgeLabelKey: "table.imageTool.badge",
        detailLabelKey: "table.imageTool.detail.yes",
      };
    case "direct_image":
      return {
        kind: "direct_image",
        showsBadge: true,
        badgeVariant: "info",
        badgeLabelKey: "table.imageTool.badge",
        detailLabelKey: "table.imageTool.detail.directImage",
      };
    case "no":
      return {
        kind: "no",
        showsBadge: false,
        badgeVariant: null,
        badgeLabelKey: null,
        detailLabelKey: "table.imageTool.detail.no",
      };
    case "unknown":
      return {
        kind: "unknown",
        showsBadge: false,
        badgeVariant: null,
        badgeLabelKey: null,
        detailLabelKey: "table.imageTool.detail.unknown",
      };
    default:
      return {
        kind: "missing",
        showsBadge: false,
        badgeVariant: null,
        badgeLabelKey: null,
        detailLabelKey: null,
      };
  }
}

export function resolveFirstResponseByteTotalMs(
  record: Pick<
    ApiInvocation,
    "tReqReadMs" | "tReqParseMs" | "tUpstreamConnectMs" | "tUpstreamTtfbMs"
  >,
): number | null {
  const stages = [
    normalizeInvocationTimingStage(record.tReqReadMs),
    normalizeInvocationTimingStage(record.tReqParseMs),
    normalizeInvocationTimingStage(record.tUpstreamConnectMs),
    normalizeInvocationTimingStage(record.tUpstreamTtfbMs),
  ];
  if (stages.some((value) => value === null)) {
    return null;
  }
  return (stages as number[]).reduce((sum, value) => sum + value, 0);
}

export function invocationStableKey(
  record: Pick<ApiInvocation, "invokeId" | "occurredAt">,
): string {
  return `${record.invokeId}-${record.occurredAt}`;
}

export function invocationStableDomKey(
  record: Pick<ApiInvocation, "invokeId" | "occurredAt"> | string,
): string {
  const stableKey = typeof record === "string" ? record : invocationStableKey(record);
  return stableKey.replace(/[^A-Za-z0-9_-]/g, "_");
}
