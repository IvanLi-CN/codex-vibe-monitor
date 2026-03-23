import type { ApiInvocation } from './api'

const DEFAULT_FALLBACK = '—'
const PRIORITY_SERVICE_TIER = 'priority'
const ROUTE_MODE_POOL = 'pool'
const RESPONSES_ENDPOINT = '/v1/responses'
const CHAT_COMPLETIONS_ENDPOINT = '/v1/chat/completions'
const COMPACT_ENDPOINT = '/v1/responses/compact'

export type ProxyWeightDeltaDirection = 'up' | 'down' | 'flat' | 'missing'
export type FastIndicatorState = 'effective' | 'requested_only' | 'none'
export type InvocationEndpointKind = 'responses' | 'chat' | 'compact' | 'raw'

type InvocationEndpointBadgeVariant = 'default' | 'secondary' | 'info'
type InvocationEndpointBadgeLabelKey =
  | 'table.endpoint.responsesBadge'
  | 'table.endpoint.chatBadge'
  | 'table.endpoint.compactBadge'

export interface ProxyWeightDeltaView {
  direction: ProxyWeightDeltaDirection
  value: string
}

export interface InvocationEndpointDisplay {
  kind: InvocationEndpointKind
  endpointValue: string
  badgeVariant: InvocationEndpointBadgeVariant | null
  labelKey: InvocationEndpointBadgeLabelKey | null
}

function normalizeInvocationTimingStage(value: number | null | undefined): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    return null
  }
  return value
}

export function normalizeServiceTier(value: string | null | undefined): string | null {
  if (typeof value !== 'string') return null
  const normalized = value.trim().toLowerCase()
  return normalized.length > 0 ? normalized : null
}

export function formatServiceTier(
  value: string | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): string {
  return normalizeServiceTier(value) ?? fallback
}

export function isPriorityServiceTier(value: string | null | undefined): boolean {
  return normalizeServiceTier(value) === PRIORITY_SERVICE_TIER
}

export function getFastIndicatorState(
  requestedServiceTier: string | null | undefined,
  effectiveServiceTier: string | null | undefined,
): FastIndicatorState {
  if (isPriorityServiceTier(effectiveServiceTier)) return 'effective'
  if (isPriorityServiceTier(requestedServiceTier)) return 'requested_only'
  return 'none'
}

export function formatProxyWeightDelta(
  value: number | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): ProxyWeightDeltaView {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return { direction: 'missing', value: fallback }
  }
  const normalized = Object.is(value, -0) ? 0 : value
  const rounded = Number(normalized.toFixed(2))
  if (rounded > 0) return { direction: 'up', value: Math.abs(rounded).toFixed(2) }
  if (rounded < 0) return { direction: 'down', value: Math.abs(rounded).toFixed(2) }
  return { direction: 'flat', value: Math.abs(rounded).toFixed(2) }
}

export function normalizeRouteMode(value: string | null | undefined): string | null {
  if (typeof value !== 'string') return null
  const normalized = value.trim().toLowerCase()
  return normalized.length > 0 ? normalized : null
}

export function isPoolRouteMode(value: string | null | undefined): boolean {
  return normalizeRouteMode(value) === ROUTE_MODE_POOL
}

export function resolveInvocationAccountLabel(
  routeMode: string | null | undefined,
  status: string | null | undefined,
  upstreamAccountName: string | null | undefined,
  upstreamAccountId: number | null | undefined,
  reverseProxyLabel: string,
  poolRoutingPendingLabel: string,
  poolAccountUnavailableLabel: string,
): string {
  if (!isPoolRouteMode(routeMode)) return reverseProxyLabel

  const name = upstreamAccountName?.trim()
  if (name) return name
  if (typeof upstreamAccountId === 'number' && Number.isFinite(upstreamAccountId)) {
    return `账号 #${Math.trunc(upstreamAccountId)}`
  }
  const normalizedStatus = status?.trim().toLowerCase()
  if (normalizedStatus === 'running' || normalizedStatus === 'pending') {
    return poolRoutingPendingLabel
  }
  return poolAccountUnavailableLabel
}

export function formatResponseContentEncoding(
  value: string | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): string {
  if (typeof value !== 'string') return fallback
  const normalized = value.trim().toLowerCase()
  return normalized.length > 0 ? normalized : fallback
}

export function resolveInvocationEndpointDisplay(
  value: string | null | undefined,
  fallback: string = DEFAULT_FALLBACK,
): InvocationEndpointDisplay {
  const endpointValue = typeof value === 'string' ? value.trim() : ''
  switch (endpointValue) {
    case RESPONSES_ENDPOINT:
      return {
        kind: 'responses',
        endpointValue,
        badgeVariant: 'default',
        labelKey: 'table.endpoint.responsesBadge',
      }
    case CHAT_COMPLETIONS_ENDPOINT:
      return {
        kind: 'chat',
        endpointValue,
        badgeVariant: 'secondary',
        labelKey: 'table.endpoint.chatBadge',
      }
    case COMPACT_ENDPOINT:
      return {
        kind: 'compact',
        endpointValue,
        badgeVariant: 'info',
        labelKey: 'table.endpoint.compactBadge',
      }
    default:
      return {
        kind: 'raw',
        endpointValue: endpointValue || fallback,
        badgeVariant: null,
        labelKey: null,
      }
  }
}

export function resolveFirstResponseByteTotalMs(
  record: Pick<
    ApiInvocation,
    'tReqReadMs' | 'tReqParseMs' | 'tUpstreamConnectMs' | 'tUpstreamTtfbMs'
  >,
): number | null {
  const stages = [
    normalizeInvocationTimingStage(record.tReqReadMs),
    normalizeInvocationTimingStage(record.tReqParseMs),
    normalizeInvocationTimingStage(record.tUpstreamConnectMs),
    normalizeInvocationTimingStage(record.tUpstreamTtfbMs),
  ]
  if (stages.some((value) => value === null)) {
    return null
  }
  return (stages as number[]).reduce((sum, value) => sum + value, 0)
}

export function invocationStableKey(record: Pick<ApiInvocation, 'invokeId' | 'occurredAt'>): string {
  return `${record.invokeId}-${record.occurredAt}`
}

export function invocationStableDomKey(
  record: Pick<ApiInvocation, 'invokeId' | 'occurredAt'> | string,
): string {
  const stableKey = typeof record === 'string' ? record : invocationStableKey(record)
  return stableKey.replace(/[^A-Za-z0-9_-]/g, '_')
}
