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
}: UpstreamAccountGroupNoteDialogProps) {
  const normalizedBoundProxyKeys = normalizeBoundProxyKeys(boundProxyKeys)
  const proxyOptions = (() => {
    const available = Array.isArray(availableProxyNodes) ? availableProxyNodes : []
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
          missing: true,
        })
      }
    }
    return options
  })()
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
                          'flex items-center gap-3 rounded-xl border px-3 py-3 text-left transition-colors',
                          selected
                            ? 'border-primary/45 bg-primary/10'
                            : 'border-base-300/80 bg-base-100/75',
                          disabled ? 'cursor-not-allowed opacity-60' : 'hover:border-primary/40',
                        )}
                      >
                        <div className="flex h-5 w-5 items-center justify-center rounded-full border border-base-300/80 bg-base-100">
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
