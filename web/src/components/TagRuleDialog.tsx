import { useEffect, useMemo, useState } from 'react'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from './ui/dialog'
import { Button } from './ui/button'
import { Input } from './ui/input'
import { SelectField } from './ui/select-field'
import { Switch } from './ui/switch'
import { ConcurrencyLimitSlider } from './ConcurrencyLimitSlider'
import type {
  CreateTagPayload,
  TagFastModeRewriteMode,
  TagPriorityTier,
  TagSummary,
  UpdateTagPayload,
} from '../lib/api'
import { apiConcurrencyLimitToSliderValue, sliderConcurrencyLimitToApiValue } from '../lib/concurrencyLimit'

export type TagRuleDialogMode = 'create' | 'edit'

type TagRuleDraft = {
  name: string
  guardEnabled: boolean
  lookbackHours: string
  maxConversations: string
  allowCutOut: boolean
  allowCutIn: boolean
  priorityTier: TagPriorityTier
  fastModeRewriteMode: TagFastModeRewriteMode
  concurrencyLimit: number
  upstream429RetryEnabled: boolean
  upstream429MaxRetries: number
}

function buildDraft(tag?: TagSummary | null, draftName = ''): TagRuleDraft {
  return {
    name: tag?.name ?? draftName,
    guardEnabled: tag?.routingRule?.guardEnabled ?? false,
    lookbackHours: tag?.routingRule?.lookbackHours == null ? '' : String(tag.routingRule.lookbackHours),
    maxConversations: tag?.routingRule?.maxConversations == null ? '' : String(tag.routingRule.maxConversations),
    allowCutOut: tag?.routingRule?.allowCutOut ?? true,
    allowCutIn: tag?.routingRule?.allowCutIn ?? true,
    priorityTier: tag?.routingRule?.priorityTier ?? 'normal',
    fastModeRewriteMode: tag?.routingRule?.fastModeRewriteMode ?? 'keep_original',
    concurrencyLimit: apiConcurrencyLimitToSliderValue(tag?.routingRule?.concurrencyLimit),
    upstream429RetryEnabled: tag?.routingRule?.upstream429RetryEnabled === true,
    upstream429MaxRetries: normalizeRetryCount(tag?.routingRule?.upstream429MaxRetries),
  }
}

function normalizePositiveInt(value: string): number | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  const parsed = Number(trimmed)
  if (!Number.isInteger(parsed) || parsed <= 0) return null
  return parsed
}

function normalizeRetryCount(value?: number | null): number {
  if (!Number.isFinite(value ?? NaN)) return 0
  return Math.max(0, Math.min(5, Math.trunc(value ?? 0)))
}

function buildPayload(
  draft: TagRuleDraft,
  options?: {
    includeName?: boolean
    changedFieldsOnly?: boolean
    baseDraft?: TagRuleDraft
  },
): CreateTagPayload | UpdateTagPayload | null {
  const lookbackHours = normalizePositiveInt(draft.lookbackHours)
  const maxConversations = normalizePositiveInt(draft.maxConversations)
  if (draft.guardEnabled && (lookbackHours == null || maxConversations == null)) return null
  const payload: UpdateTagPayload = {
    guardEnabled: draft.guardEnabled,
    lookbackHours: draft.guardEnabled ? lookbackHours : undefined,
    maxConversations: draft.guardEnabled ? maxConversations : undefined,
    allowCutOut: draft.allowCutOut,
    allowCutIn: draft.allowCutIn,
    priorityTier: draft.priorityTier,
    fastModeRewriteMode: draft.fastModeRewriteMode,
    concurrencyLimit: sliderConcurrencyLimitToApiValue(draft.concurrencyLimit),
    upstream429RetryEnabled: draft.upstream429RetryEnabled,
    upstream429MaxRetries: draft.upstream429RetryEnabled
      ? Math.max(1, normalizeRetryCount(draft.upstream429MaxRetries) || 1)
      : 0,
  }
  if (options?.includeName !== false) {
    return {
      ...payload,
      name: draft.name.trim(),
    }
  }
  if (options?.changedFieldsOnly && options.baseDraft) {
    const base = options.baseDraft
    const changedPayload: UpdateTagPayload = {}
    if (
      draft.guardEnabled !== base.guardEnabled ||
      draft.lookbackHours !== base.lookbackHours ||
      draft.maxConversations !== base.maxConversations
    ) {
      changedPayload.guardEnabled = payload.guardEnabled
      if (payload.guardEnabled) {
        changedPayload.lookbackHours = payload.lookbackHours
        changedPayload.maxConversations = payload.maxConversations
      }
    }
    if (draft.allowCutOut !== base.allowCutOut) {
      changedPayload.allowCutOut = payload.allowCutOut
    }
    if (draft.allowCutIn !== base.allowCutIn) {
      changedPayload.allowCutIn = payload.allowCutIn
    }
    if (draft.priorityTier !== base.priorityTier) {
      changedPayload.priorityTier = payload.priorityTier
    }
    if (draft.fastModeRewriteMode !== base.fastModeRewriteMode) {
      changedPayload.fastModeRewriteMode = payload.fastModeRewriteMode
    }
    if (draft.concurrencyLimit !== base.concurrencyLimit) {
      changedPayload.concurrencyLimit = payload.concurrencyLimit
    }
    if (
      draft.upstream429RetryEnabled !== base.upstream429RetryEnabled ||
      draft.upstream429MaxRetries !== base.upstream429MaxRetries
    ) {
      changedPayload.upstream429RetryEnabled = payload.upstream429RetryEnabled
      changedPayload.upstream429MaxRetries = payload.upstream429MaxRetries
    }
    return changedPayload
  }
  return payload
}

interface TagRuleDialogProps {
  open: boolean
  mode: TagRuleDialogMode
  tag?: TagSummary | null
  draftName?: string
  busy?: boolean
  error?: string | null
  policyOnly?: boolean
  changedFieldsOnly?: boolean
  title?: string
  description?: string
  submitLabel?: string
  onClose: () => void
  onSubmit: (payload: CreateTagPayload | UpdateTagPayload) => Promise<void> | void
  labels: {
    createTitle: string
    editTitle: string
    description: string
    name: string
    namePlaceholder: string
    guardEnabled: string
    forbidNewConversation?: string
    lookbackHours: string
    maxConversations: string
    allowCutOut: string
    allowCutIn: string
    forbidCutOut?: string
    forbidCutIn?: string
    priorityTier: string
    priorityPrimary: string
    priorityNormal: string
    priorityFallback: string
    fastModeRewriteMode: string
    fastModeKeepOriginal: string
    fastModeFillMissing: string
    fastModeForceAdd: string
    fastModeForceRemove: string
    concurrencyLimit?: string
    concurrencyHint?: string
    currentValue?: string
    unlimited?: string
    upstream429Retry?: string
    upstream429RetryHint?: string
    upstream429RetryToggle?: string
    upstream429RetryCount?: string
    upstream429RetryCountOnce?: string
    upstream429RetryCountMany?: (count: number) => string
    cancel: string
    save: string
    create: string
    validation: string
  }
}

export function TagRuleDialog({
  open,
  mode,
  tag,
  draftName,
  busy = false,
  error,
  policyOnly = false,
  changedFieldsOnly = false,
  title,
  description,
  submitLabel,
  onClose,
  onSubmit,
  labels,
}: TagRuleDialogProps) {
  const [draft, setDraft] = useState<TagRuleDraft>(() => buildDraft(tag, draftName))
  const baseDraft = useMemo(() => buildDraft(tag, draftName), [draftName, tag])

  useEffect(() => {
    if (open) setDraft(baseDraft)
  }, [baseDraft, open])

  const payload = useMemo(
    () =>
      buildPayload(draft, {
        includeName: !policyOnly,
        changedFieldsOnly,
        baseDraft,
      }),
    [baseDraft, changedFieldsOnly, draft, policyOnly],
  )
  const disabled = !payload || (!policyOnly && !draft.name.trim()) || busy

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy && !nextOpen ? onClose() : undefined)}>
      <DialogContent className="p-0">
        <div className="border-b border-base-300/80 px-6 py-5">
          <DialogHeader>
            <DialogTitle>{title ?? (mode === 'create' ? labels.createTitle : labels.editTitle)}</DialogTitle>
            <DialogDescription>{description ?? labels.description}</DialogDescription>
          </DialogHeader>
        </div>
        <div className="space-y-5 px-6 py-5">
          {!policyOnly ? (
            <label className="field">
              <span className="field-label">{labels.name}</span>
              <Input
                name="tagName"
                value={draft.name}
                placeholder={labels.namePlaceholder}
                onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))}
              />
            </label>
          ) : null}

          <SelectField
            className="field"
            label={labels.priorityTier}
            name="tagPriorityTier"
            value={draft.priorityTier}
            disabled={busy}
            options={[
              { value: 'primary', label: labels.priorityPrimary },
              { value: 'normal', label: labels.priorityNormal },
              { value: 'fallback', label: labels.priorityFallback },
            ]}
            onValueChange={(value) => setDraft((current) => ({ ...current, priorityTier: value as TagPriorityTier }))}
          />

          <SelectField
            className="field"
            label={labels.fastModeRewriteMode}
            name="tagFastModeRewriteMode"
            value={draft.fastModeRewriteMode}
            disabled={busy}
            options={[
              { value: 'keep_original', label: labels.fastModeKeepOriginal },
              { value: 'fill_missing', label: labels.fastModeFillMissing },
              { value: 'force_add', label: labels.fastModeForceAdd },
              { value: 'force_remove', label: labels.fastModeForceRemove },
            ]}
            onValueChange={(value) =>
              setDraft((current) => ({
                ...current,
                fastModeRewriteMode: value as TagFastModeRewriteMode,
              }))}
          />

          <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
            <div className="flex items-center justify-between gap-4">
              <div>
                <p className="font-medium text-base-content">
                  {labels.forbidNewConversation ?? labels.guardEnabled}
                </p>
              </div>
              <Switch checked={draft.guardEnabled} onCheckedChange={(checked) => setDraft((current) => ({ ...current, guardEnabled: checked }))} />
            </div>
            <div className="mt-4 grid gap-3 sm:grid-cols-2">
              <label className="field">
                <span className="field-label">{labels.lookbackHours}</span>
                <Input
                  name="tagLookbackHours"
                  value={draft.lookbackHours}
                  inputMode="numeric"
                  disabled={!draft.guardEnabled}
                  onChange={(event) => setDraft((current) => ({ ...current, lookbackHours: event.target.value }))}
                />
              </label>
              <label className="field">
                <span className="field-label">{labels.maxConversations}</span>
                <Input
                  name="tagMaxConversations"
                  value={draft.maxConversations}
                  inputMode="numeric"
                  disabled={!draft.guardEnabled}
                  onChange={(event) => setDraft((current) => ({ ...current, maxConversations: event.target.value }))}
                />
              </label>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
              <div className="flex items-center justify-between gap-4">
                <p className="font-medium text-base-content">
                  {labels.forbidCutOut ?? labels.allowCutOut}
                </p>
                <Switch
                  checked={!draft.allowCutOut}
                  onCheckedChange={(checked) =>
                    setDraft((current) => ({ ...current, allowCutOut: !checked }))}
                />
              </div>
            </div>
            <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
              <div className="flex items-center justify-between gap-4">
                <p className="font-medium text-base-content">
                  {labels.forbidCutIn ?? labels.allowCutIn}
                </p>
                <Switch
                  checked={!draft.allowCutIn}
                  onCheckedChange={(checked) =>
                    setDraft((current) => ({ ...current, allowCutIn: !checked }))}
                />
              </div>
            </div>
          </div>

          <ConcurrencyLimitSlider
            value={draft.concurrencyLimit}
            disabled={busy}
            title={labels.concurrencyLimit ?? 'Concurrency limit'}
            description={labels.concurrencyHint ?? 'Use 1-30 to cap fresh assignments. The last step means unlimited.'}
            currentLabel={labels.currentValue ?? 'Current'}
            unlimitedLabel={labels.unlimited ?? 'Unlimited'}
            onChange={(value) => setDraft((current) => ({ ...current, concurrencyLimit: value }))}
          />

          <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
            <div className="flex items-start justify-between gap-4">
              <div className="space-y-1">
                <p className="font-medium text-base-content">{labels.upstream429Retry ?? 'Upstream 429 retry'}</p>
                <p className="text-xs leading-5 text-base-content/65">
                  {labels.upstream429RetryHint ?? 'Retry the same upstream account before cooldown and failover.'}
                </p>
              </div>
              <Switch
                checked={draft.upstream429RetryEnabled}
                onCheckedChange={(checked) =>
                  setDraft((current) => ({
                    ...current,
                    upstream429RetryEnabled: checked,
                    upstream429MaxRetries: checked ? Math.max(1, current.upstream429MaxRetries || 1) : 0,
                  }))}
                aria-label={labels.upstream429RetryToggle ?? 'Retry after upstream 429'}
              />
            </div>
            <SelectField
              className="mt-4"
              label={labels.upstream429RetryCount ?? 'Retry count'}
              name="tagUpstream429MaxRetries"
              value={String(Math.max(1, draft.upstream429MaxRetries || 1))}
              disabled={busy || !draft.upstream429RetryEnabled}
              options={[1, 2, 3, 4, 5].map((value) => ({
                value: String(value),
                label: value === 1
                  ? labels.upstream429RetryCountOnce ?? '1 retry'
                  : labels.upstream429RetryCountMany?.(value) ?? `${value} retries`,
              }))}
              onValueChange={(value) =>
                setDraft((current) => ({
                  ...current,
                  upstream429MaxRetries: normalizeRetryCount(Number(value)),
                }))}
            />
          </div>

          {error ? <p className="text-sm text-error">{error}</p> : null}
          {!payload && draft.guardEnabled ? <p className="text-sm text-warning">{labels.validation}</p> : null}
        </div>
        <div className="border-t border-base-300/80 px-6 py-4">
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>{labels.cancel}</Button>
            <Button
              type="button"
              disabled={disabled}
              onClick={() => {
                if (payload) void onSubmit(payload)
              }}
            >
              {submitLabel ?? (mode === 'create' ? labels.create : labels.save)}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  )
}
