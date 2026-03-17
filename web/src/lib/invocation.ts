import type { ApiInvocation } from './api'

const DEFAULT_FALLBACK = '—'
const PRIORITY_SERVICE_TIER = 'priority'
const ROUTE_MODE_POOL = 'pool'

export type ProxyWeightDeltaDirection = 'up' | 'down' | 'flat' | 'missing'
export type FastIndicatorState = 'effective' | 'requested_only' | 'none'

export interface ProxyWeightDeltaView {
  direction: ProxyWeightDeltaDirection
  value: string
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

export function invocationStableKey(record: Pick<ApiInvocation, 'invokeId' | 'occurredAt'>): string {
  return `${record.invokeId}-${record.occurredAt}`
}

export function invocationStableDomKey(
  record: Pick<ApiInvocation, 'invokeId' | 'occurredAt'> | string,
): string {
  const stableKey = typeof record === 'string' ? record : invocationStableKey(record)
  return stableKey.replace(/[^A-Za-z0-9_-]/g, '_')
}
