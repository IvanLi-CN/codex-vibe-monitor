import { useMemo } from 'react'
import { AppIcon } from './AppIcon'
import type { ForwardProxyBindingNode } from '../lib/api'
import { cn } from '../lib/utils'
import { Button } from './ui/button'
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog'

type GroupProxyOption = ForwardProxyBindingNode & {
  missing?: boolean
}

interface UpstreamAccountGroupNoteDialogProps {
  open: boolean
  container?: HTMLElement | null
  groupName: string
  note: string
  busy?: boolean
  error?: string | null
  existing: boolean
  boundProxyKeys?: string[]
  availableProxyNodes?: ForwardProxyBindingNode[]
  onNoteChange: (value: string) => void
  onBoundProxyKeysChange?: (value: string[]) => void
  onClose: () => void
  onSave: () => void
  title: string
  existingDescription: string
  draftDescription: string
  noteLabel: string
  notePlaceholder: string
  cancelLabel: string
  saveLabel: string
  closeLabel: string
  existingBadgeLabel: string
  draftBadgeLabel: string
  proxyBindingsLabel?: string
  proxyBindingsHint?: string
  proxyBindingsAutomaticLabel?: string
  proxyBindingsEmptyLabel?: string
  proxyBindingsMissingLabel?: string
  proxyBindingsUnavailableLabel?: string
  proxyBindingsChartLabel?: string
  proxyBindingsChartSuccessLabel?: string
  proxyBindingsChartFailureLabel?: string
  proxyBindingsChartEmptyLabel?: string
}

function normalizeBoundProxyKeys(values?: string[]): string[] {
  if (!Array.isArray(values)) return []
  return Array.from(new Set(values.map((value) => value.trim()).filter((value) => value.length > 0)))
}

function toggleBoundProxyKey(keys: string[], target: string): string[] {
  if (keys.includes(target)) {
    return keys.filter((key) => key !== target)
  }
  return [...keys, target]
}

function buildVisibleBarHeights(successCount: number, failureCount: number, scaleMax: number, totalHeightPx: number) {
  if (scaleMax <= 0 || totalHeightPx <= 0) {
    return { empty: totalHeightPx, failure: 0, success: 0 }
  }

  let success = successCount > 0 ? Math.max((successCount / scaleMax) * totalHeightPx, 1) : 0
  let failure = failureCount > 0 ? Math.max((failureCount / scaleMax) * totalHeightPx, 1) : 0
  const maxVisible = Math.max(totalHeightPx, 0)
  let overflow = success + failure - maxVisible

  const shrink = (value: number, minVisible: number, amount: number) => {
    if (amount <= 0 || value <= minVisible) return { nextValue: value, remaining: amount }
    const delta = Math.min(value - minVisible, amount)
    return { nextValue: value - delta, remaining: amount - delta }
  }

  if (overflow > 0) {
    const first = success >= failure ? 'success' : 'failure'
    const second = first == 'success' ? 'failure' : 'success'
    for (const key of [first, second] as const) {
      const minVisible = key == 'success' ? (successCount > 0 ? 1 : 0) : failureCount > 0 ? 1 : 0
      const current = key == 'success' ? success : failure
      const result = shrink(current, minVisible, overflow)
      if (key == 'success') {
        success = result.nextValue
      } else {
        failure = result.nextValue
      }
      overflow = result.remaining
    }
  }

  const used = Math.min(success + failure, maxVisible)
  return {
    empty: Math.max(maxVisible - used, 0),
    failure,
    success,
  }
}

function sumProxyTraffic(node: ForwardProxyBindingNode) {
  const buckets = Array.isArray(node.last24h) ? node.last24h : []
  return buckets.reduce(
    (acc, bucket) => {
      acc.success += bucket.successCount
      acc.failure += bucket.failureCount
      return acc
    },
    { success: 0, failure: 0 },
  )
}

function ProxyOptionTrafficChart({
  node,
  scaleMax,
  label,
  successLabel,
  failureLabel,
  emptyLabel,
}: {
  node: ForwardProxyBindingNode
  scaleMax: number
  label: string
  successLabel: string
  failureLabel: string
  emptyLabel: string
}) {
  const buckets = useMemo(() => (Array.isArray(node.last24h) ? node.last24h : []), [node.last24h])
  const totals = useMemo(() => sumProxyTraffic(node), [node])

  return (
    <div className="w-full md:w-[13.5rem] md:max-w-[13.5rem]">
      <div className="flex items-center justify-between gap-3">
        <span className="text-[10px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
          {label}
        </span>
        <div className="flex items-center gap-3 text-[11px] font-medium">
          <span className="inline-flex items-center gap-1 text-success">
            <span className="h-1.5 w-1.5 rounded-full bg-success" aria-hidden />
            <span className="font-mono tabular-nums">{totals.success}</span>
            <span className="text-base-content/55">{successLabel}</span>
          </span>
          <span className="inline-flex items-center gap-1 text-error">
            <span className="h-1.5 w-1.5 rounded-full bg-error" aria-hidden />
            <span className="font-mono tabular-nums">{totals.failure}</span>
            <span className="text-base-content/55">{failureLabel}</span>
          </span>
        </div>
      </div>

      {buckets.length === 0 ? (
        <div className="mt-2 flex h-12 items-center justify-center rounded-xl border border-dashed border-base-300/80 bg-base-100/70 px-3 text-[11px] text-base-content/50">
          {emptyLabel}
        </div>
      ) : (
        <div
          role="img"
          aria-label={`${node.displayName} ${label}: ${totals.success} ${successLabel}, ${totals.failure} ${failureLabel}`}
          className="mt-2 flex h-12 items-end gap-px rounded-xl border border-base-300/70 bg-base-100/70 px-2 py-1.5"
          data-chart-kind="proxy-binding-request-trend"
        >
          {buckets.map((bucket) => {
            const total = bucket.successCount + bucket.failureCount
            const heights = buildVisibleBarHeights(bucket.successCount, bucket.failureCount, scaleMax, 32)
            return (
              <div
                key={`${node.key}-${bucket.bucketStart}`}
                className="flex h-8 min-w-0 flex-1 flex-col overflow-hidden rounded-[3px] bg-base-300/35"
              >
                <div className="bg-transparent" style={{ height: `${heights.empty}px` }} />
                <div className={cn(total > 0 ? 'bg-error/85' : 'bg-transparent')} style={{ height: `${heights.failure}px` }} />
                <div className={cn(total > 0 ? 'bg-success/85' : 'bg-transparent')} style={{ height: `${heights.success}px` }} />
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

export function UpstreamAccountGroupNoteDialog({
  open,
  container,
  groupName,
  note,
  busy = false,
  error,
  existing,
  boundProxyKeys,
  availableProxyNodes,
  onNoteChange,
  onBoundProxyKeysChange,
  onClose,
  onSave,
  title,
  existingDescription,
  draftDescription,
  noteLabel,
  notePlaceholder,
  cancelLabel,
  saveLabel,
  closeLabel,
  existingBadgeLabel,
  draftBadgeLabel,
  proxyBindingsLabel,
  proxyBindingsHint,
  proxyBindingsAutomaticLabel,
  proxyBindingsEmptyLabel,
  proxyBindingsMissingLabel,
  proxyBindingsUnavailableLabel,
  proxyBindingsChartLabel,
  proxyBindingsChartSuccessLabel,
  proxyBindingsChartFailureLabel,
  proxyBindingsChartEmptyLabel,
}: UpstreamAccountGroupNoteDialogProps) {
  const normalizedBoundProxyKeys = normalizeBoundProxyKeys(boundProxyKeys)
  const proxyOptions = (() => {
    const available = Array.isArray(availableProxyNodes)
      ? availableProxyNodes.map((node) => ({
          ...node,
          last24h: Array.isArray(node.last24h) ? node.last24h : [],
        }))
      : []
    const availableByKey = new Map(available.map((node) => [node.key, node]))
    const options: GroupProxyOption[] = [...available]
    for (const key of normalizedBoundProxyKeys) {
      if (!availableByKey.has(key)) {
        options.push({
          key,
          source: 'missing',
          displayName: key,
          penalized: false,
          selectable: false,
          last24h: [],
          missing: true,
        })
      }
    }
    return options
  })()
  const proxyChartScaleMax = useMemo(
    () =>
      Math.max(
        ...proxyOptions.flatMap((node) =>
          (Array.isArray(node.last24h) ? node.last24h : []).map((bucket) => bucket.successCount + bucket.failureCount),
        ),
        0,
      ),
    [proxyOptions],
  )
  const showProxyBindings =
    Boolean(onBoundProxyKeysChange) ||
    proxyOptions.length > 0 ||
    normalizedBoundProxyKeys.length > 0

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy ? (nextOpen ? undefined : onClose()) : undefined)}>
      <DialogContent container={container} className="overflow-hidden border-base-300 bg-base-100 p-0">
        <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
          <DialogHeader className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <DialogTitle>{title}</DialogTitle>
              <span className="rounded-full border border-base-300/80 bg-base-200/80 px-2.5 py-1 text-xs font-semibold text-base-content/70">
                {existing ? existingBadgeLabel : draftBadgeLabel}
              </span>
            </div>
            <DialogDescription>
              {existing ? existingDescription : draftDescription}
            </DialogDescription>
            <p className="text-sm font-semibold text-base-content">{groupName}</p>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} disabled={busy} />
        </div>

        <div className="grid gap-4 px-6 py-5">
          {error ? (
            <div className="flex items-start gap-3 rounded-2xl border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
              <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{error}</div>
            </div>
          ) : null}

          <label className="field">
            <span className="field-label">{noteLabel}</span>
            <textarea
              className="min-h-32 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
              value={note}
              placeholder={notePlaceholder}
              disabled={busy}
              onChange={(event) => onNoteChange(event.target.value)}
            />
          </label>

          {showProxyBindings ? (
            <section className="space-y-3 rounded-2xl border border-base-300/80 bg-base-200/25 px-4 py-4">
              <div className="space-y-1">
                <h3 className="text-sm font-semibold text-base-content">
                  {proxyBindingsLabel ?? 'Bound proxy nodes'}
                </h3>
                <p className="text-xs leading-5 text-base-content/68">
                  {proxyBindingsHint ?? 'Leave empty to use automatic routing.'}
                </p>
              </div>

              {normalizedBoundProxyKeys.length === 0 ? (
                <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65">
                  {proxyBindingsAutomaticLabel ?? 'No nodes bound. This group uses automatic routing.'}
                </div>
              ) : null}

              {proxyOptions.length === 0 ? (
                <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/65 px-3 py-2 text-xs text-base-content/65">
                  {proxyBindingsEmptyLabel ?? 'No proxy nodes available.'}
                </div>
              ) : (
                <div className="grid gap-2">
                  {proxyOptions.map((node) => {
                    const selected = normalizedBoundProxyKeys.includes(node.key)
                    const disabled = busy || (!selected && !node.selectable)
                    const badgeLabel = node.missing
                      ? proxyBindingsMissingLabel ?? 'Missing'
                      : !node.selectable
                        ? proxyBindingsUnavailableLabel ?? 'Unavailable'
                        : null
                    return (
                      <button
                        key={node.key}
                        type="button"
                        disabled={disabled}
                        onClick={() => {
                          if (!onBoundProxyKeysChange) return
                          onBoundProxyKeysChange(toggleBoundProxyKey(normalizedBoundProxyKeys, node.key))
                        }}
                        className={cn(
                          'flex flex-col gap-3 rounded-xl border px-3 py-3 text-left transition-colors md:flex-row md:items-center md:justify-between',
                          selected
                            ? 'border-primary/45 bg-primary/10'
                            : 'border-base-300/80 bg-base-100/75',
                          disabled ? 'cursor-not-allowed opacity-60' : 'hover:border-primary/40',
                        )}
                      >
                        <div className="flex min-w-0 flex-1 items-center gap-3">
                          <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-base-300/80 bg-base-100">
                            {selected ? <AppIcon name="check" className="h-3.5 w-3.5 text-primary" aria-hidden /> : null}
                          </div>
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-2">
                              <span className="truncate text-sm font-medium text-base-content">{node.displayName}</span>
                              {badgeLabel ? (
                                <span className="rounded-full border border-base-300/80 bg-base-200/80 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/65">
                                  {badgeLabel}
                                </span>
                              ) : null}
                              {node.penalized ? (
                                <span className="rounded-full border border-warning/35 bg-warning/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-warning">
                                  Penalized
                                </span>
                              ) : null}
                            </div>
                            <div className="mt-1 text-xs text-base-content/62">
                              {node.key}
                            </div>
                          </div>
                        </div>
                        <ProxyOptionTrafficChart
                          node={node}
                          scaleMax={proxyChartScaleMax}
                          label={proxyBindingsChartLabel ?? '24h request trend'}
                          successLabel={proxyBindingsChartSuccessLabel ?? 'ok'}
                          failureLabel={proxyBindingsChartFailureLabel ?? 'fail'}
                          emptyLabel={proxyBindingsChartEmptyLabel ?? 'No 24h data'}
                        />
                      </button>
                    )
                  })}
                </div>
              )}
            </section>
          ) : null}
        </div>

        <DialogFooter className="border-t border-base-300/80 px-6 py-5">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button type="button" onClick={onSave} disabled={busy}>
            {busy ? <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden /> : null}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
