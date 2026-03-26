import { Fragment, type ReactNode, useCallback, useEffect, useMemo, useState } from 'react'
import { AppIcon } from './AppIcon'
import type { ApiInvocation } from '../lib/api'
import {
  invocationStableKey,
  invocationStableDomKey,
  type FastIndicatorState,
  type InvocationEndpointDisplay,
} from '../lib/invocation'
import { resolveInvocationDisplayStatus } from '../lib/invocationStatus'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { InvocationAccountDetailDrawer } from './InvocationAccountDetailDrawer'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Spinner } from './ui/spinner'
import { cn } from '../lib/utils'
import {
  FALLBACK_CELL,
  InvocationExpandedDetails,
  buildInvocationDetailViewModel,
  renderEndpointSummary,
  renderFastIndicator,
  renderReasoningEffortBadge,
  useInvocationPoolAttempts,
} from './invocation-details-shared'

interface InvocationTableProps {
  records: ApiInvocation[]
  isLoading: boolean
  error?: string | null
  emptyLabel?: string
}

const STATUS_META: Record<
  string,
  { variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'; key: TranslationKey }
> = {
  success: { variant: 'success', key: 'table.status.success' },
  completed: { variant: 'success', key: 'table.status.success' },
  failed: { variant: 'error', key: 'table.status.failed' },
  running: { variant: 'default', key: 'table.status.running' },
  pending: { variant: 'warning', key: 'table.status.pending' },
}

const FALLBACK_STATUS_META: {
  variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'
  key: TranslationKey
} = { variant: 'secondary', key: 'table.status.unknown' }

function resolveStatusMeta(
  status: string,
): { variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'; key: TranslationKey } {
  const normalized = status.trim().toLowerCase()
  if (!normalized || normalized === 'unknown') {
    return FALLBACK_STATUS_META
  }

  return STATUS_META[normalized] ?? STATUS_META.failed
}

interface InvocationRowViewModel {
  record: ApiInvocation
  rowKey: string
  recordId: number
  meta: { variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'; key: TranslationKey }
  isInFlight: boolean
  occurredTime: string
  occurredDate: string
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
  responseContentEncodingValue: string
  detailNotice: string | null
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>
  timingPairs: Array<{ label: string; value: string }>
}

export function InvocationTable({ records, isLoading, error, emptyLabel }: InvocationTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [drawerAccountId, setDrawerAccountId] = useState<number | null>(null)
  const [drawerAccountLabel, setDrawerAccountLabel] = useState<string | null>(null)
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [isXlUp, setIsXlUp] = useState(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false
    return window.matchMedia('(min-width: 1280px)').matches
  })

  const toggleLabels = useMemo(() => {
    if (locale === 'zh') {
      return {
        header: '详情',
        show: '展开详情',
        hide: '收起详情',
        expanded: '已展开',
        collapsed: '未展开',
      }
    }
    return {
      header: 'Details',
      show: 'Show details',
      hide: 'Hide details',
      expanded: 'Expanded',
      collapsed: 'Collapsed',
    }
  }, [locale])

  const openAccountDrawer = (accountId: number | null, accountLabel: string) => {
    if (accountId == null) return
    setDrawerAccountId(accountId)
    setDrawerAccountLabel(accountLabel)
  }

  const closeAccountDrawer = () => {
    setDrawerAccountId(null)
    setDrawerAccountLabel(null)
  }

  const renderAccountValue = useCallback(
    (
      accountLabel: string,
      accountId: number | null,
      accountClickable: boolean,
      className?: string,
    ) => {
      if (!accountClickable || accountId == null) {
        return (
          <span
            className={cn(
              'inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap leading-none',
              className,
            )}
            title={accountLabel}
          >
            {accountLabel}
          </span>
        )
      }

      return (
        <button
          type="button"
          className={cn(
            'inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap appearance-none border-0 bg-transparent p-0 align-middle font-inherit leading-none text-center text-current no-underline shadow-none transition hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary',
            className,
          )}
          onClick={() => openAccountDrawer(accountId, accountLabel)}
          title={accountLabel}
        >
          {accountLabel}
        </button>
      )
    },
    [],
  )

  useEffect(() => {
    setExpandedId((current) => {
      if (current === null) return current
      return records.some((record) => invocationStableKey(record) === current) ? current : null
    })
  }, [records])

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
    const mediaQuery = window.matchMedia('(min-width: 1280px)')
    const sync = () => {
      setIsXlUp(mediaQuery.matches)
    }

    sync()
    if (typeof mediaQuery.addEventListener === 'function') {
      mediaQuery.addEventListener('change', sync)
      return () => {
        mediaQuery.removeEventListener('change', sync)
      }
    }

    mediaQuery.addListener(sync)
    return () => {
      mediaQuery.removeListener(sync)
    }
  }, [])

  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: '2-digit',
        day: '2-digit',
      }),
    [localeTag],
  )
  const timeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
      }),
    [localeTag],
  )
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: 'currency',
        currency: 'USD',
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  )

  const rows = useMemo<InvocationRowViewModel[]>(
    () =>
      records.map((record) => {
        const rowKey = invocationStableKey(record)
        const occurred = new Date(record.occurredAt)
        const normalizedStatus = (resolveInvocationDisplayStatus(record) || 'unknown').toLowerCase()
        const meta = resolveStatusMeta(normalizedStatus)
        const recordId = record.id
        const isInFlight = normalizedStatus === 'running' || normalizedStatus === 'pending'
        const occurredValid = !Number.isNaN(occurred.getTime())
        const occurredTime = occurredValid ? timeFormatter.format(occurred) : record.occurredAt
        const occurredDate = occurredValid ? dateFormatter.format(occurred) : FALLBACK_CELL
        const detailView = buildInvocationDetailViewModel({
          record,
          normalizedStatus,
          t,
          locale,
          localeTag,
          nowMs,
          numberFormatter,
          currencyFormatter,
          renderAccountValue,
        })

        return {
          record,
          rowKey,
          recordId,
          meta,
          isInFlight,
          occurredTime,
          occurredDate,
          ...detailView,
        }
      }),
    [records, currencyFormatter, dateFormatter, locale, localeTag, nowMs, numberFormatter, renderAccountValue, t, timeFormatter],
  )

  const hasInFlightRows = useMemo(() => rows.some((row) => row.isInFlight), [rows])
  const expandedRecord = useMemo(
    () => rows.find((row) => row.rowKey === expandedId)?.record ?? null,
    [expandedId, rows],
  )
  const poolAttemptsState = useInvocationPoolAttempts(expandedRecord)

  useEffect(() => {
    if (!hasInFlightRows) return
    setNowMs(Date.now())
    const id = window.setInterval(() => {
      setNowMs(Date.now())
    }, 1000)
    return () => window.clearInterval(id)
  }, [hasInFlightRows])

  if (error) {
    return (
      <Alert variant="error">
        <span>{t('table.loadError', { error })}</span>
      </Alert>
    )
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t('table.loadingRecordsAria')} />
      </div>
    )
  }

  if (records.length === 0) {
    return <Alert>{emptyLabel ?? t('table.noRecords')}</Alert>
  }

  return (
    <div className="space-y-3">
      <div className="space-y-3 md:hidden" data-testid="invocation-list">
        {rows.map((row, rowIndex) => {
          const listDetailId = `invocation-list-details-${invocationStableDomKey(row.rowKey)}`
          const isExpanded = expandedId === row.rowKey
          const handleToggle = () => {
            setExpandedId((current) => (current === row.rowKey ? null : row.rowKey))
          }

          return (
            <article
              key={`mobile-${row.rowKey}`}
              data-testid="invocation-list-item"
              className={`rounded-xl border border-base-300/70 px-3 py-3 ${rowIndex % 2 === 0 ? 'bg-base-100/40' : 'bg-base-200/24'}`}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-semibold">{row.occurredTime}</div>
                  <div className="truncate text-xs text-base-content/65">{row.occurredDate}</div>
                </div>
                <button
                  type="button"
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                  onClick={handleToggle}
                  aria-expanded={isExpanded}
                  aria-controls={listDetailId}
                  aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                >
                  <AppIcon
                    name={isExpanded ? 'chevron-down' : 'chevron-right'}
                    className="h-5 w-5"
                    aria-hidden
                  />
                  <span className="sr-only">{isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}</span>
                </button>
              </div>

              <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
                <Badge variant={row.meta.variant}>{t(row.meta.key)}</Badge>
                <div className="min-w-0 flex-1">
                  <div data-testid="invocation-account-name">
                    {renderAccountValue(row.accountLabel, row.accountId, row.accountClickable, 'text-xs font-medium text-base-content')}
                  </div>
                  <div
                    className="min-w-0 truncate text-[11px] text-base-content/70"
                    title={row.proxyDisplayName}
                    data-testid="invocation-proxy-name"
                  >
                    {row.proxyDisplayName}
                  </div>
                </div>
              </div>

              <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs font-mono text-base-content/70">
                <span title={row.totalLatencyValue}>{`${t('table.column.totalLatencyShort')} ${row.totalLatencyValue}`}</span>
                <span title={row.firstResponseByteTotalValue}>{`${t('table.column.firstResponseByteTotalShort')} ${row.firstResponseByteTotalValue}`}</span>
                <span title={row.responseContentEncodingValue}>{`${t('table.column.httpCompressionShort')} ${row.responseContentEncodingValue}`}</span>
              </div>

              <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                <dt className="text-base-content/65">{t('table.column.model')}</dt>
                <dd className="min-w-0">
                  <div className="flex items-start justify-end gap-1 text-right" title={row.modelValue}>
                    <span className="min-w-0 flex-1 truncate">{row.modelValue}</span>
                    {renderFastIndicator(row.fastIndicatorState, t)}
                  </div>
                </dd>
                <dt className="text-base-content/65">{t('table.column.costUsd')}</dt>
                <dd className="truncate text-right font-mono">{row.costValue}</dd>
                <dt className="text-base-content/65">{t('table.column.inputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.inputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.cacheInputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.cacheInputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.outputTokens')}</dt>
                <dd className="text-right">
                  <div className="flex flex-col items-end gap-0.5 leading-tight">
                    <span className="truncate font-mono">{row.outputTokensValue}</span>
                    <span
                      className="truncate text-[11px] text-base-content/70"
                      title={`${t('table.details.reasoningTokens')}: ${row.reasoningTokensValue}`}
                    >
                      {row.outputReasoningBreakdownValue}
                    </span>
                  </div>
                </dd>
                <dt className="text-base-content/65">{t('table.column.totalTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.totalTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.reasoningEffort')}</dt>
                <dd className="flex justify-end">{renderReasoningEffortBadge(row.reasoningEffortValue)}</dd>
              </dl>

              <div className="mt-3 space-y-1 border-t border-base-300/65 pt-2">
                <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">{t('table.details.endpoint')}</div>
                {renderEndpointSummary(row.endpointDisplay, t, 'text-xs')}
                <div className="truncate text-xs" title={row.errorMessage || undefined}>{row.errorMessage || FALLBACK_CELL}</div>
              </div>

              {isExpanded && (
                <div className="mt-3 rounded-lg border border-base-300/70 bg-base-200/58">
                  <InvocationExpandedDetails
                    record={row.record}
                    detailId={listDetailId}
                    detailPairs={row.detailPairs}
                    timingPairs={row.timingPairs}
                    errorMessage={row.errorMessage}
                    detailNotice={row.detailNotice}
                    size="compact"
                    poolAttemptsState={poolAttemptsState}
                    t={t}
                  />
                </div>
              )}
            </article>
          )
        })}
      </div>

      <div className="hidden md:block">
        <div
          className="overflow-x-hidden rounded-xl border border-base-300/70 bg-base-100/52 backdrop-blur"
          data-testid="invocation-table-scroll"
        >
          <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
            <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
              <tr>
                <th className="w-[11%] px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:w-[10%] xl:px-3">{t('table.column.time')}</th>
                <th className="w-[18%] px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:w-[15%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.account')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.proxy')}
                    </span>
                  </div>
                </th>
                <th className="w-[13%] px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:w-[12%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.latency')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.firstResponseByteTotalCompression')}
                    </span>
                  </div>
                </th>
                <th className="w-[17%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[14%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.model')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.costUsd')}
                    </span>
                  </div>
                </th>
                <th className="w-[16%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[14%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.inputTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.cacheInputTokens')}
                    </span>
                  </div>
                </th>
                <th className="w-[10%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[10%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.outputTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.details.reasoningTokens')}
                    </span>
                  </div>
                </th>
                <th className="w-[12%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[11%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.totalTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.reasoningEffort')}
                    </span>
                  </div>
                </th>
                <th className="hidden w-[10%] px-2 py-2.5 text-left font-semibold xl:table-cell xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.error')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.details.endpoint')}
                    </span>
                  </div>
                </th>
                <th className="w-[9%] px-2 py-2.5 text-right xl:w-[4%] xl:px-3">
                  <span className="sr-only">{toggleLabels.header}</span>
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-base-300/65">
              {rows.map((row, rowIndex) => {
                const tableDetailId = `invocation-table-details-${invocationStableDomKey(row.rowKey)}`
                const isExpanded = expandedId === row.rowKey
                const handleToggle = () => {
                  setExpandedId((current) => (current === row.rowKey ? null : row.rowKey))
                }

                return (
                  <Fragment key={row.rowKey}>
                    <tr className={`${rowIndex % 2 === 0 ? 'bg-base-100/38' : 'bg-base-200/22'} hover:bg-primary/6`}>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <span className="truncate whitespace-nowrap font-medium">{row.occurredTime}</span>
                          <span className="truncate whitespace-nowrap text-base-content/70">{row.occurredDate}</span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-center justify-center gap-1 leading-tight text-center">
                          <Badge
                            variant={row.meta.variant}
                            className="mx-auto h-6 w-fit max-w-full items-center justify-center overflow-hidden px-2.5 py-0 text-[11px] font-semibold text-center leading-none"
                            data-testid="invocation-proxy-badge"
                          >
                            <span
                              className="inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap leading-none"
                              data-testid="invocation-account-name"
                            >
                              {renderAccountValue(row.accountLabel, row.accountId, row.accountClickable)}
                            </span>
                            <span className="sr-only">{t(row.meta.key)}</span>
                          </Badge>
                          <span
                            className="block w-full truncate whitespace-nowrap text-center text-[11px] text-base-content/70"
                            title={row.proxyDisplayName}
                            data-testid="invocation-proxy-name"
                          >
                            {row.proxyDisplayName}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <span className="truncate whitespace-nowrap font-mono tabular-nums" title={row.totalLatencyValue}>
                            {row.totalLatencyValue}
                          </span>
                          <span
                            className="truncate whitespace-nowrap text-[11px] text-base-content/70"
                            title={`${row.firstResponseByteTotalValue} · ${row.responseContentEncodingValue}`}
                          >
                            {`${row.firstResponseByteTotalValue} · ${row.responseContentEncodingValue}`}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <div className="flex w-full items-start justify-end gap-1">
                            <span className="min-w-0 flex-1 truncate whitespace-nowrap text-base-content/85" title={row.modelValue}>
                              {row.modelValue}
                            </span>
                            {renderFastIndicator(row.fastIndicatorState, t)}
                          </div>
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                            {row.costValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums">
                            {row.inputTokensValue}
                          </span>
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                            {row.cacheInputTokensValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="block w-full truncate whitespace-nowrap font-mono tabular-nums">{row.outputTokensValue}</span>
                          <span
                            className="block w-full truncate whitespace-nowrap text-[11px] text-base-content/70"
                            title={`${t('table.details.reasoningTokens')}: ${row.reasoningTokensValue}`}
                          >
                            {row.outputReasoningBreakdownValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="block w-full truncate whitespace-nowrap font-mono tabular-nums">{row.totalTokensValue}</span>
                          <div className="flex w-full justify-end">{renderReasoningEffortBadge(row.reasoningEffortValue)}</div>
                        </div>
                      </td>
                      <td className="hidden min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:table-cell xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          {renderEndpointSummary(row.endpointDisplay, t)}
                          <span className="block truncate whitespace-nowrap" title={row.errorMessage || undefined}>
                            {row.errorMessage || FALLBACK_CELL}
                          </span>
                        </div>
                      </td>
                      <td className="border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <button
                          type="button"
                          className="inline-flex items-center justify-end gap-1 text-lg leading-none text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                          onClick={handleToggle}
                          aria-expanded={isExpanded}
                          aria-controls={tableDetailId}
                          aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                        >
                          <AppIcon
                            name={isExpanded ? 'chevron-down' : 'chevron-right'}
                            className="h-4 w-4"
                            aria-hidden
                          />
                          <span className="sr-only">{isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}</span>
                        </button>
                      </td>
                    </tr>
                    {isExpanded && (
                      <tr className="bg-base-200/68">
                        <td colSpan={isXlUp ? 9 : 8} className="border-t border-base-300/65 px-2 py-2.5 xl:px-3">
                          <InvocationExpandedDetails
                            record={row.record}
                            detailId={tableDetailId}
                            detailPairs={row.detailPairs}
                            timingPairs={row.timingPairs}
                            errorMessage={row.errorMessage}
                            detailNotice={row.detailNotice}
                            size="default"
                            poolAttemptsState={poolAttemptsState}
                            t={t}
                          />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                )
              })}
            </tbody>
          </table>
        </div>
      </div>
      <InvocationAccountDetailDrawer
        open={drawerAccountId != null}
        accountId={drawerAccountId}
        accountLabel={drawerAccountLabel}
        onClose={closeAccountDrawer}
      />
    </div>
  )
}
