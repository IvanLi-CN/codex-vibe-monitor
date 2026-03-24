/* eslint-disable react-refresh/only-export-components */
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Badge } from './ui/badge'
import { Spinner } from './ui/spinner'
import {
  fetchInvocationPoolAttempts,
  type ApiInvocation,
  type ApiPoolUpstreamRequestAttempt,
} from '../lib/api'
import {
  formatProxyWeightDelta,
  formatResponseContentEncoding,
  formatServiceTier,
  getFastIndicatorState,
  isPoolRouteMode,
  resolveFirstResponseByteTotalMs,
  resolveInvocationAccountLabel,
  resolveInvocationEndpointDisplay,
  type FastIndicatorState,
  type InvocationEndpointDisplay,
} from '../lib/invocation'
import type { TranslationKey } from '../i18n'
import { cn } from '../lib/utils'
import { getReasoningEffortTone, REASONING_EFFORT_TONE_CLASSNAMES } from './invocation-table-reasoning'

export const FALLBACK_CELL = '—'

type Translator = (key: TranslationKey, values?: Record<string, string | number>) => string

export type DetailPanelSize = 'compact' | 'default'

export interface InvocationDetailViewModel {
  accountLabel: string
  accountId: number | null
  accountClickable: boolean
  proxyDisplayName: string
  modelValue: string
  requestedServiceTierValue: string
  serviceTierValue: string
  fastIndicatorState: FastIndicatorState
  costValue: string
  inputTokensValue: string
  cacheInputTokensValue: string
  outputTokensValue: string
  outputReasoningBreakdownValue: string
  reasoningTokensValue: string
  reasoningEffortValue: string
  totalTokensValue: string
  endpointValue: string
  endpointDisplay: InvocationEndpointDisplay
  errorMessage: string
  totalLatencyValue: string
  firstResponseByteTotalValue: string
  firstByteLatencyValue: string
  responseContentEncodingValue: string
  detailNotice: string | null
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>
  timingPairs: Array<{ label: string; value: string }>
}

export interface InvocationPoolAttemptsState {
  attemptsByInvokeId: Record<string, ApiPoolUpstreamRequestAttempt[] | undefined>
  loadingByInvokeId: Record<string, boolean | undefined>
  errorByInvokeId: Record<string, string | null | undefined>
}

export type InvocationAccountValueRenderer = (
  accountLabel: string,
  accountId: number | null,
  accountClickable: boolean,
  className?: string,
) => ReactNode

interface BuildInvocationDetailViewModelOptions {
  record: ApiInvocation
  normalizedStatus: string
  t: Translator
  locale: string
  localeTag: string
  nowMs: number
  numberFormatter: Intl.NumberFormat
  currencyFormatter: Intl.NumberFormat
  renderAccountValue: InvocationAccountValueRenderer
}

interface InvocationExpandedDetailsProps {
  record: ApiInvocation
  detailId: string
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>
  timingPairs: Array<{ label: string; value: string }>
  errorMessage: string
  detailNotice: string | null
  size: DetailPanelSize
  poolAttemptsState: InvocationPoolAttemptsState
  t: Translator
}

function isZhLocale(locale: string) {
  return locale.trim().toLowerCase().startsWith('zh')
}

export function formatMilliseconds(value: number | null | undefined) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return `${value.toFixed(1)} ms`
}

export function formatSecondsFromMilliseconds(value: number | null | undefined, localeTag: string) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL

  const seconds = value / 1000
  const precision = Math.abs(seconds) >= 100 ? 1 : Math.abs(seconds) >= 1 ? 2 : 3
  const rounded = Number(seconds.toFixed(precision))

  return `${rounded.toLocaleString(localeTag, {
    minimumFractionDigits: 0,
    maximumFractionDigits: precision,
  })} s`
}

export function formatElapsedSecondsFromTimestamp(
  occurredAt: string | null | undefined,
  localeTag: string,
  nowMs: number,
) {
  const occurredMs = occurredAt ? Date.parse(occurredAt) : Number.NaN
  if (!Number.isFinite(occurredMs)) return FALLBACK_CELL
  return formatSecondsFromMilliseconds(Math.max(0, nowMs - occurredMs), localeTag)
}

export function formatOptionalNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return formatter.format(value)
}

export function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim()
  return normalized ? normalized : FALLBACK_CELL
}

export function canOpenInvocationAccount(record: ApiInvocation) {
  return (
    isPoolRouteMode(record.routeMode) &&
    typeof record.upstreamAccountId === 'number' &&
    Number.isFinite(record.upstreamAccountId)
  )
}

function normalizeDetailLevel(value: ApiInvocation['detailLevel']) {
  return value === 'structured_only' ? 'structured_only' : 'full'
}

export function formatDetailTimestamp(value: string | null | undefined) {
  const normalized = value?.trim()
  if (!normalized) return FALLBACK_CELL

  const parsed = new Date(normalized)
  if (Number.isNaN(parsed.getTime())) return normalized

  return parsed.toISOString().replace('.000Z', 'Z').replace('T', ' ')
}

export function renderReasoningEffortBadge(value: string) {
  if (value === FALLBACK_CELL) {
    return <span className="font-mono text-sm text-base-content/70">{FALLBACK_CELL}</span>
  }

  const tone = getReasoningEffortTone(value)

  return (
    <Badge
      variant="secondary"
      className={cn(
        'max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]',
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={value}
      data-reasoning-effort-tone={tone}
    >
      <span className="block max-w-full truncate whitespace-nowrap">{value}</span>
    </Badge>
  )
}

export function resolveProxyDisplayName(record: ApiInvocation) {
  const payloadProxyName = record.proxyDisplayName?.trim()
  if (payloadProxyName) return payloadProxyName
  const sourceValue = record.source?.trim()
  if (sourceValue && sourceValue.toLowerCase() !== 'proxy') return sourceValue
  return FALLBACK_CELL
}

export function renderFastIndicator(state: FastIndicatorState, t: Translator) {
  if (state === 'none') return null

  const isEffective = state === 'effective'
  const titleKey: TranslationKey = isEffective
    ? 'table.model.fastPriorityTitle'
    : 'table.model.fastRequestedOnlyTitle'
  const ariaKey: TranslationKey = isEffective
    ? 'table.model.fastPriorityAria'
    : 'table.model.fastRequestedOnlyAria'

  return (
    <span
      className={cn(
        'mt-0.5 inline-flex h-3.5 w-3.5 flex-none',
        isEffective ? 'text-amber-500' : 'text-base-content/50',
      )}
      title={t(titleKey)}
      aria-label={t(ariaKey)}
      data-testid="invocation-fast-icon"
      data-fast-state={state}
      role="img"
    >
      <AppIcon name="lightning-bolt" className="h-3.5 w-3.5" aria-hidden />
    </span>
  )
}

function renderEndpointRawPath(endpointValue: string, className?: string) {
  return (
    <span
      className={cn('block truncate whitespace-nowrap font-mono text-base-content/70', className)}
      title={endpointValue}
      data-testid="invocation-endpoint-path"
      data-endpoint-kind="raw"
    >
      {endpointValue}
    </span>
  )
}

export function renderEndpointSummary(
  endpointDisplay: InvocationEndpointDisplay,
  t: Translator,
  className?: string,
) {
  if (endpointDisplay.kind === 'raw' || endpointDisplay.labelKey == null || endpointDisplay.badgeVariant == null) {
    return renderEndpointRawPath(endpointDisplay.endpointValue, className)
  }

  const title =
    endpointDisplay.kind === 'compact'
      ? `${t('table.endpoint.compactHint')} · ${endpointDisplay.endpointValue}`
      : endpointDisplay.endpointValue

  return (
    <Badge
      variant={endpointDisplay.badgeVariant}
      className={cn(
        'invocation-endpoint-badge max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]',
        className,
      )}
      title={title}
      data-testid="invocation-endpoint-badge"
      data-endpoint-kind={endpointDisplay.kind}
    >
      <span className="block max-w-full truncate whitespace-nowrap">{t(endpointDisplay.labelKey)}</span>
    </Badge>
  )
}

function formatFailureClassValue(
  failureClass: ApiInvocation['failureClass'],
  t: Translator,
): ReactNode {
  switch (failureClass) {
    case 'service_failure':
      return <Badge variant="error">{t('records.filters.failureClass.service')}</Badge>
    case 'client_failure':
      return <Badge variant="warning">{t('records.filters.failureClass.client')}</Badge>
    case 'client_abort':
      return <Badge variant="secondary">{t('records.filters.failureClass.abort')}</Badge>
    default:
      return FALLBACK_CELL
  }
}

function formatActionableValue(value: ApiInvocation['isActionable'], t: Translator) {
  if (typeof value !== 'boolean') return FALLBACK_CELL
  return (
    <Badge variant={value ? 'warning' : 'secondary'}>
      {value ? t('table.details.actionableYes') : t('table.details.actionableNo')}
    </Badge>
  )
}

function resolveDetailLabels(locale: string) {
  if (isZhLocale(locale)) {
    return {
      full: 'Full',
      structuredOnly: 'Structured only',
      level: '细节层级',
      prunedAt: '精简时间',
      pruneReason: '精简原因',
      fullHint: '完整调试细节仍在当前在线保留窗口内。',
      structuredHint: '该记录仅保留结构化字段；离线归档保留归档行，超窗 raw file 不保证继续可用。',
      prunedPrefix: '精简于',
    }
  }

  return {
    full: 'Full',
    structuredOnly: 'Structured only',
    level: 'Detail level',
    prunedAt: 'Detail pruned at',
    pruneReason: 'Detail prune reason',
    fullHint: 'Full troubleshooting detail is still available inside the online retention window.',
    structuredHint: 'Only structured fields remain online for this record. Offline archives keep the archived row, but aged raw files may no longer be available.',
    prunedPrefix: 'Pruned at',
  }
}

function renderDetailEndpointValue(
  endpointDisplay: InvocationEndpointDisplay,
  endpointValue: string,
  t: Translator,
) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="w-fit max-w-full">{renderEndpointSummary(endpointDisplay, t)}</div>
      <span className="break-all font-mono text-xs text-base-content/70">{endpointValue}</span>
    </div>
  )
}

export function buildInvocationDetailViewModel({
  record,
  normalizedStatus,
  t,
  locale,
  localeTag,
  nowMs,
  numberFormatter,
  currencyFormatter,
  renderAccountValue,
}: BuildInvocationDetailViewModelOptions): InvocationDetailViewModel {
  const proxyDisplayName = resolveProxyDisplayName(record)
  const accountLabel = resolveInvocationAccountLabel(
    record.routeMode,
    normalizedStatus,
    record.upstreamAccountName,
    record.upstreamAccountId,
    t('table.account.reverseProxy'),
    t('table.account.poolRoutingPending'),
    t('table.account.poolAccountUnavailable'),
  )
  const accountClickable = canOpenInvocationAccount(record)
  const requestedServiceTierValue = formatServiceTier(record.requestedServiceTier)
  const serviceTierValue = formatServiceTier(record.serviceTier)
  const fastIndicatorState = getFastIndicatorState(record.requestedServiceTier, record.serviceTier)
  const reasoningEffortValue = formatOptionalText(record.reasoningEffort)
  const reasoningTokensValue = formatOptionalNumber(record.reasoningTokens, numberFormatter)
  const outputReasoningBreakdownValue = `${t('table.column.reasoningTokensShort')} ${reasoningTokensValue}`
  const totalLatencyValue =
    normalizedStatus === 'running' || normalizedStatus === 'pending'
      ? formatElapsedSecondsFromTimestamp(record.occurredAt, localeTag, nowMs)
      : formatSecondsFromMilliseconds(record.tTotalMs, localeTag)
  const firstResponseByteTotalValue = formatSecondsFromMilliseconds(
    resolveFirstResponseByteTotalMs(record),
    localeTag,
  )
  const firstByteLatencyValue = formatMilliseconds(record.tUpstreamTtfbMs)
  const responseContentEncodingValue = formatResponseContentEncoding(record.responseContentEncoding)
  const endpointDisplay = resolveInvocationEndpointDisplay(record.endpoint)
  const endpointValue = endpointDisplay.endpointValue
  const errorMessage = record.errorMessage?.trim() ?? ''

  const proxyWeightDeltaView = formatProxyWeightDelta(record.proxyWeightDelta)
  const proxyWeightDeltaValue =
    proxyWeightDeltaView.direction === 'missing' ? (
      FALLBACK_CELL
    ) : (
      <span
        className={`inline-flex items-center gap-1 font-mono ${
          proxyWeightDeltaView.direction === 'up'
            ? 'text-success'
            : proxyWeightDeltaView.direction === 'down'
              ? 'text-error'
              : 'text-base-content/70'
        }`}
        aria-label={
          proxyWeightDeltaView.direction === 'up'
            ? t('table.details.proxyWeightDeltaA11yIncrease', { value: proxyWeightDeltaView.value })
            : proxyWeightDeltaView.direction === 'down'
              ? t('table.details.proxyWeightDeltaA11yDecrease', { value: proxyWeightDeltaView.value })
              : t('table.details.proxyWeightDeltaA11yUnchanged', { value: proxyWeightDeltaView.value })
        }
      >
        <AppIcon
          name={
            proxyWeightDeltaView.direction === 'up'
              ? 'arrow-up-bold'
              : proxyWeightDeltaView.direction === 'down'
                ? 'arrow-down-bold'
                : 'arrow-right-bold'
          }
          className="h-3.5 w-3.5"
          aria-hidden
        />
        <span aria-hidden>{proxyWeightDeltaView.value}</span>
      </span>
    )

  const detailLabels = resolveDetailLabels(locale)
  const detailLevel = normalizeDetailLevel(record.detailLevel)
  const detailPrunedAtValue = formatDetailTimestamp(record.detailPrunedAt)
  const detailPruneReasonValue = formatOptionalText(record.detailPruneReason)
  const detailLevelBadgeLabel = detailLevel === 'structured_only' ? detailLabels.structuredOnly : detailLabels.full
  const detailLevelBadgeVariant = detailLevel === 'structured_only' ? 'warning' : 'secondary'
  const detailNotice = detailLevel === 'structured_only' ? detailLabels.structuredHint : null
  const detailPrunedSummary =
    detailLevel === 'structured_only' && detailPrunedAtValue !== FALLBACK_CELL
      ? `${detailLabels.prunedPrefix} ${detailPrunedAtValue}`
      : null
  const detailLevelTooltip =
    detailLevel === 'structured_only'
      ? detailPrunedSummary
        ? `${detailLabels.structuredHint} ${detailPrunedSummary}.`
        : detailLabels.structuredHint
      : detailLabels.fullHint

  const detailPairs: Array<{ key: string; label: string; value: ReactNode }> = [
    { key: 'invokeId', label: t('table.details.invokeId'), value: record.invokeId || FALLBACK_CELL },
    { key: 'source', label: t('table.details.source'), value: record.source || FALLBACK_CELL },
    {
      key: 'account',
      label: t('table.details.account'),
      value: renderAccountValue(accountLabel, record.upstreamAccountId ?? null, accountClickable, 'font-mono text-sm'),
    },
    { key: 'proxy', label: t('table.details.proxy'), value: proxyDisplayName },
    {
      key: 'endpoint',
      label: t('table.details.endpoint'),
      value: renderDetailEndpointValue(endpointDisplay, endpointValue, t),
    },
    { key: 'requesterIp', label: t('table.details.requesterIp'), value: record.requesterIp || FALLBACK_CELL },
    { key: 'promptCacheKey', label: t('table.details.promptCacheKey'), value: record.promptCacheKey || FALLBACK_CELL },
    {
      key: 'poolAttemptCount',
      label: t('table.details.poolAttemptCount'),
      value: formatOptionalText(record.poolAttemptCount != null ? String(record.poolAttemptCount) : undefined),
    },
    {
      key: 'poolDistinctAccountCount',
      label: t('table.details.poolDistinctAccountCount'),
      value: formatOptionalText(
        record.poolDistinctAccountCount != null ? String(record.poolDistinctAccountCount) : undefined,
      ),
    },
    {
      key: 'poolAttemptTerminalReason',
      label: t('table.details.poolAttemptTerminalReason'),
      value: formatOptionalText(record.poolAttemptTerminalReason),
    },
    { key: 'totalLatency', label: t('table.details.totalLatency'), value: totalLatencyValue },
    {
      key: 'firstResponseByteTotal',
      label: t('table.details.firstResponseByteTotal'),
      value: firstResponseByteTotalValue,
    },
    { key: 'firstByteLatency', label: t('table.details.firstByteLatency'), value: firstByteLatencyValue },
    { key: 'responseContentEncoding', label: t('table.details.httpCompression'), value: responseContentEncodingValue },
    { key: 'requestedServiceTier', label: t('table.details.requestedServiceTier'), value: requestedServiceTierValue },
    { key: 'serviceTier', label: t('table.details.serviceTier'), value: serviceTierValue },
    { key: 'reasoningEffort', label: t('table.details.reasoningEffort'), value: renderReasoningEffortBadge(reasoningEffortValue) },
    { key: 'reasoningTokens', label: t('table.details.reasoningTokens'), value: reasoningTokensValue },
    { key: 'proxyWeightDelta', label: t('table.details.proxyWeightDelta'), value: proxyWeightDeltaValue },
    { key: 'failureClass', label: t('table.details.failureClass'), value: formatFailureClassValue(record.failureClass, t) },
    { key: 'actionable', label: t('table.details.actionable'), value: formatActionableValue(record.isActionable, t) },
    { key: 'failureKind', label: t('table.details.failureKind'), value: formatOptionalText(record.failureKind) },
    { key: 'streamTerminalEvent', label: t('table.details.streamTerminalEvent'), value: formatOptionalText(record.streamTerminalEvent) },
    { key: 'upstreamErrorCode', label: t('table.details.upstreamErrorCode'), value: formatOptionalText(record.upstreamErrorCode) },
    { key: 'upstreamErrorMessage', label: t('table.details.upstreamErrorMessage'), value: formatOptionalText(record.upstreamErrorMessage) },
    { key: 'upstreamRequestId', label: t('table.details.upstreamRequestId'), value: formatOptionalText(record.upstreamRequestId) },
    {
      key: 'detailLevel',
      label: detailLabels.level,
      value: (
        <Badge
          variant={detailLevelBadgeVariant}
          className="max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]"
          title={detailLevelTooltip}
          data-testid="invocation-detail-level-badge"
        >
          <span className="block max-w-full truncate whitespace-nowrap">{detailLevelBadgeLabel}</span>
        </Badge>
      ),
    },
    { key: 'detailPrunedAt', label: detailLabels.prunedAt, value: detailPrunedAtValue },
    { key: 'detailPruneReason', label: detailLabels.pruneReason, value: detailPruneReasonValue },
  ]

  const timingPairs: Array<{ label: string; value: string }> = [
    { label: t('table.details.stage.requestRead'), value: formatSecondsFromMilliseconds(record.tReqReadMs, localeTag) },
    { label: t('table.details.stage.requestParse'), value: formatSecondsFromMilliseconds(record.tReqParseMs, localeTag) },
    { label: t('table.details.stage.upstreamConnect'), value: formatSecondsFromMilliseconds(record.tUpstreamConnectMs, localeTag) },
    { label: t('table.details.stage.upstreamFirstByte'), value: formatMilliseconds(record.tUpstreamTtfbMs) },
    { label: t('table.details.stage.upstreamStream'), value: formatSecondsFromMilliseconds(record.tUpstreamStreamMs, localeTag) },
    { label: t('table.details.stage.responseParse'), value: formatSecondsFromMilliseconds(record.tRespParseMs, localeTag) },
    { label: t('table.details.stage.persistence'), value: formatSecondsFromMilliseconds(record.tPersistMs, localeTag) },
    { label: t('table.details.stage.total'), value: formatSecondsFromMilliseconds(record.tTotalMs, localeTag) },
  ]

  return {
    accountLabel,
    accountId:
      typeof record.upstreamAccountId === 'number' && Number.isFinite(record.upstreamAccountId)
        ? Math.trunc(record.upstreamAccountId)
        : null,
    accountClickable,
    proxyDisplayName,
    modelValue: record.model ?? FALLBACK_CELL,
    requestedServiceTierValue,
    serviceTierValue,
    fastIndicatorState,
    costValue: typeof record.cost === 'number' ? currencyFormatter.format(record.cost) : FALLBACK_CELL,
    inputTokensValue: formatOptionalNumber(record.inputTokens, numberFormatter),
    cacheInputTokensValue: formatOptionalNumber(record.cacheInputTokens, numberFormatter),
    outputTokensValue: formatOptionalNumber(record.outputTokens, numberFormatter),
    outputReasoningBreakdownValue,
    reasoningTokensValue,
    reasoningEffortValue,
    totalTokensValue: formatOptionalNumber(record.totalTokens, numberFormatter),
    endpointValue,
    endpointDisplay,
    errorMessage,
    totalLatencyValue,
    firstResponseByteTotalValue,
    firstByteLatencyValue,
    responseContentEncodingValue,
    detailNotice,
    detailPairs,
    timingPairs,
  }
}

function formatPoolAttemptAccountLabel(attempt: ApiPoolUpstreamRequestAttempt) {
  const accountName = attempt.upstreamAccountName?.trim()
  if (accountName) return accountName
  if (typeof attempt.upstreamAccountId === 'number' && Number.isFinite(attempt.upstreamAccountId)) {
    return `#${Math.trunc(attempt.upstreamAccountId)}`
  }
  return FALLBACK_CELL
}

function poolAttemptStatusMeta(
  status: string | null | undefined,
): { variant: 'success' | 'warning' | 'error' | 'secondary'; key: TranslationKey } {
  switch (status?.trim().toLowerCase()) {
    case 'success':
      return { variant: 'success', key: 'table.poolAttempts.status.success' }
    case 'http_failure':
      return { variant: 'error', key: 'table.poolAttempts.status.httpFailure' }
    case 'transport_failure':
      return { variant: 'warning', key: 'table.poolAttempts.status.transportFailure' }
    case 'budget_exhausted_final':
      return { variant: 'error', key: 'table.poolAttempts.status.budgetExhaustedFinal' }
    default:
      return { variant: 'secondary', key: 'table.poolAttempts.status.unknown' }
  }
}

export function useInvocationPoolAttempts(expandedRecord: ApiInvocation | null) {
  const [attemptsByInvokeId, setPoolAttemptsByInvokeId] = useState<
    Record<string, ApiPoolUpstreamRequestAttempt[] | undefined>
  >({})
  const [loadingByInvokeId, setPoolAttemptLoadingByInvokeId] = useState<Record<string, boolean | undefined>>({})
  const [errorByInvokeId, setPoolAttemptErrorByInvokeId] = useState<Record<string, string | null | undefined>>({})
  const attemptsRef = useRef(attemptsByInvokeId)
  const loadingRef = useRef(loadingByInvokeId)
  const loadedKeyRef = useRef<Record<string, string | undefined>>({})
  const loadingKeyRef = useRef<Record<string, string | undefined>>({})
  const activeRequestIdRef = useRef<Record<string, number | undefined>>({})
  const nextRequestIdRef = useRef(0)

  useEffect(() => {
    attemptsRef.current = attemptsByInvokeId
  }, [attemptsByInvokeId])

  useEffect(() => {
    loadingRef.current = loadingByInvokeId
  }, [loadingByInvokeId])

  useEffect(() => {
    if (!expandedRecord || !isPoolRouteMode(expandedRecord.routeMode)) return
    const invokeId = expandedRecord.invokeId
    const normalizedStatus = expandedRecord.status?.trim().toLowerCase() ?? ''
    const requestKey = [
      normalizedStatus,
      expandedRecord.poolAttemptCount ?? '',
      expandedRecord.poolDistinctAccountCount ?? '',
      expandedRecord.poolAttemptTerminalReason ?? '',
      expandedRecord.failureKind ?? '',
      expandedRecord.errorMessage ?? '',
      expandedRecord.upstreamErrorCode ?? '',
      expandedRecord.upstreamErrorMessage ?? '',
      expandedRecord.upstreamRequestId ?? '',
      expandedRecord.upstreamAccountId ?? '',
      expandedRecord.upstreamAccountName ?? '',
      expandedRecord.tUpstreamConnectMs ?? '',
      expandedRecord.tUpstreamTtfbMs ?? '',
      expandedRecord.tUpstreamStreamMs ?? '',
    ].join('|')
    const isInFlight = normalizedStatus === 'running' || normalizedStatus === 'pending'
    const hasCachedAttempts = attemptsRef.current[invokeId] !== undefined
    const loadedKey = loadedKeyRef.current[invokeId]
    const loadingKey = loadingKeyRef.current[invokeId]

    if (loadingRef.current[invokeId] && loadingKey === requestKey) return
    if (hasCachedAttempts && loadedKey === requestKey && !isInFlight) return

    let cancelled = false
    const requestId = ++nextRequestIdRef.current
    loadingKeyRef.current[invokeId] = requestKey
    activeRequestIdRef.current[invokeId] = requestId
    setPoolAttemptLoadingByInvokeId((current) => ({ ...current, [invokeId]: true }))
    setPoolAttemptErrorByInvokeId((current) => ({ ...current, [invokeId]: null }))

    fetchInvocationPoolAttempts(invokeId)
      .then((attempts) => {
        if (cancelled) return
        loadedKeyRef.current[invokeId] = requestKey
        setPoolAttemptsByInvokeId((current) => ({ ...current, [invokeId]: attempts }))
      })
      .catch((error) => {
        if (cancelled) return
        const message = error instanceof Error ? error.message : String(error)
        setPoolAttemptErrorByInvokeId((current) => ({ ...current, [invokeId]: message }))
      })
      .finally(() => {
        if (activeRequestIdRef.current[invokeId] === requestId) {
          delete activeRequestIdRef.current[invokeId]
          delete loadingKeyRef.current[invokeId]
          setPoolAttemptLoadingByInvokeId((current) => ({ ...current, [invokeId]: false }))
        }
      })

    return () => {
      cancelled = true
      if (activeRequestIdRef.current[invokeId] === requestId) {
        delete activeRequestIdRef.current[invokeId]
        delete loadingKeyRef.current[invokeId]
        setPoolAttemptLoadingByInvokeId((current) => ({ ...current, [invokeId]: false }))
      }
    }
  }, [
    expandedRecord?.invokeId,
    expandedRecord?.routeMode,
    expandedRecord?.status,
    expandedRecord?.poolAttemptCount,
    expandedRecord?.poolDistinctAccountCount,
    expandedRecord?.poolAttemptTerminalReason,
    expandedRecord?.failureKind,
    expandedRecord?.errorMessage,
    expandedRecord?.upstreamErrorCode,
    expandedRecord?.upstreamErrorMessage,
    expandedRecord?.upstreamRequestId,
    expandedRecord?.upstreamAccountId,
    expandedRecord?.upstreamAccountName,
    expandedRecord?.tUpstreamConnectMs,
    expandedRecord?.tUpstreamTtfbMs,
    expandedRecord?.tUpstreamStreamMs,
  ])

  return {
    attemptsByInvokeId,
    loadingByInvokeId,
    errorByInvokeId,
  }
}

function renderPoolAttemptsContent(
  record: ApiInvocation,
  poolAttemptsState: InvocationPoolAttemptsState,
  t: Translator,
) {
  const invokeId = record.invokeId
  const attempts = poolAttemptsState.attemptsByInvokeId[invokeId]
  const isLoadingAttempts = !!poolAttemptsState.loadingByInvokeId[invokeId]
  const attemptsError = poolAttemptsState.errorByInvokeId[invokeId]

  if (!isPoolRouteMode(record.routeMode)) {
    return (
      <div
        className="rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
        data-testid="pool-attempts-empty"
      >
        {t('table.poolAttempts.notPool')}
      </div>
    )
  }

  const summaryParts = [
    `${t('table.details.poolAttemptCount')}: ${formatOptionalText(
      record.poolAttemptCount != null ? String(record.poolAttemptCount) : undefined,
    )}`,
    `${t('table.details.poolDistinctAccountCount')}: ${formatOptionalText(
      record.poolDistinctAccountCount != null ? String(record.poolDistinctAccountCount) : undefined,
    )}`,
    `${t('table.details.poolAttemptTerminalReason')}: ${formatOptionalText(record.poolAttemptTerminalReason)}`,
  ]

  return (
    <div className="flex flex-col gap-3" data-testid="pool-attempts-section">
      <div className="space-y-1">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t('table.poolAttempts.title')}
        </span>
        <div className="text-xs text-base-content/60">{summaryParts.join(' · ')}</div>
      </div>

      {attemptsError ? (
        <div
          className="rounded-lg border border-error/25 bg-error/8 px-3 py-2 text-sm text-error"
          data-testid="pool-attempts-error"
        >
          {t('table.poolAttempts.loadError', { error: attemptsError })}
        </div>
      ) : attempts && attempts.length > 0 ? (
        <div className="space-y-2" data-testid="pool-attempts-list">
          {attempts.map((attempt) => {
            const statusMeta = poolAttemptStatusMeta(attempt.status)
            const accountLabel = formatPoolAttemptAccountLabel(attempt)
            const httpStatusValue =
              typeof attempt.httpStatus === 'number' && Number.isFinite(attempt.httpStatus)
                ? String(Math.trunc(attempt.httpStatus))
                : FALLBACK_CELL

            return (
              <div
                key={`${attempt.id}-${attempt.attemptIndex}`}
                className="rounded-lg border border-base-300/70 bg-base-100/70 p-3"
                data-testid="pool-attempt-item"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant={statusMeta.variant}>{t(statusMeta.key)}</Badge>
                  <span className="font-mono text-xs text-base-content/70">#{attempt.attemptIndex}</span>
                  <span className="text-sm font-medium">{accountLabel}</span>
                </div>
                <div className="mt-2 grid gap-2 text-sm md:grid-cols-2 xl:grid-cols-3">
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.retry')}
                    </span>
                    <span className="font-mono">
                      {attempt.sameAccountRetryIndex}/{attempt.distinctAccountIndex}
                    </span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.httpStatus')}
                    </span>
                    <span className="font-mono">{httpStatusValue}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.failureKind')}
                    </span>
                    <span className="break-all font-mono">{formatOptionalText(attempt.failureKind)}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.connectLatency')}
                    </span>
                    <span className="font-mono">{formatMilliseconds(attempt.connectLatencyMs)}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.firstByteLatency')}
                    </span>
                    <span className="font-mono">{formatMilliseconds(attempt.firstByteLatencyMs)}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.streamLatency')}
                    </span>
                    <span className="font-mono">{formatMilliseconds(attempt.streamLatencyMs)}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.startedAt')}
                    </span>
                    <span className="font-mono">{formatDetailTimestamp(attempt.startedAt)}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.finishedAt')}
                    </span>
                    <span className="font-mono">{formatDetailTimestamp(attempt.finishedAt)}</span>
                  </div>
                  <div className="flex items-start gap-2">
                    <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60">
                      {t('table.poolAttempts.upstreamRequestId')}
                    </span>
                    <span className="break-all font-mono">{formatOptionalText(attempt.upstreamRequestId)}</span>
                  </div>
                </div>
                {attempt.errorMessage?.trim() ? (
                  <pre className="mt-2 whitespace-pre-wrap break-words font-mono text-sm text-base-content/80">
                    {attempt.errorMessage}
                  </pre>
                ) : null}
              </div>
            )
          })}
        </div>
      ) : isLoadingAttempts ? (
        <div
          className="inline-flex items-center gap-2 rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
          data-testid="pool-attempts-loading"
        >
          <Spinner size="sm" aria-label={t('table.poolAttempts.loading')} />
          <span>{t('table.poolAttempts.loading')}</span>
        </div>
      ) : (
        <div
          className="rounded-lg border border-base-300/70 bg-base-200/45 px-3 py-2 text-sm text-base-content/70"
          data-testid="pool-attempts-empty"
        >
          {t('table.poolAttempts.empty')}
        </div>
      )}
    </div>
  )
}

export function InvocationExpandedDetails({
  record,
  detailId,
  detailPairs,
  timingPairs,
  errorMessage,
  detailNotice,
  size,
  poolAttemptsState,
  t,
}: InvocationExpandedDetailsProps) {
  return (
    <div id={detailId} className={cn('flex flex-col gap-4', size === 'compact' ? 'p-3' : 'p-4')}>
      {detailNotice ? (
        <div
          className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs leading-5 text-warning"
          data-testid="invocation-detail-notice"
        >
          {detailNotice}
        </div>
      ) : null}

      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t('table.detailsTitle')}
        </span>
        <div className="grid gap-2 md:grid-cols-2">
          {detailPairs.map((entry) => (
            <div key={entry.key} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">
                {entry.label}
              </span>
              <div className="min-w-0 break-all font-mono text-sm">{entry.value}</div>
            </div>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t('table.details.timingsTitle')}
        </span>
        <div className="grid gap-2 md:grid-cols-2">
          {timingPairs.map((entry) => (
            <div key={entry.label} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">
                {entry.label}
              </span>
              <span className="font-mono text-sm">{entry.value}</span>
            </div>
          ))}
        </div>
      </div>

      {renderPoolAttemptsContent(record, poolAttemptsState, t)}

      {errorMessage ? (
        <div className="flex flex-col gap-2">
          <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
            {t('table.errorDetailsTitle')}
          </span>
          <pre className="whitespace-pre-wrap break-words font-mono text-sm">{errorMessage}</pre>
        </div>
      ) : null}
    </div>
  )
}
