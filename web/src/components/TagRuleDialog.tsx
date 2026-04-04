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
}

function buildDraft(tag?: TagSummary | null, draftName = ''): TagRuleDraft {
  return {
    name: tag?.name ?? draftName,
    guardEnabled: tag?.routingRule.guardEnabled ?? false,
    lookbackHours: tag?.routingRule.lookbackHours == null ? '' : String(tag.routingRule.lookbackHours),
    maxConversations: tag?.routingRule.maxConversations == null ? '' : String(tag.routingRule.maxConversations),
    allowCutOut: tag?.routingRule.allowCutOut ?? true,
    allowCutIn: tag?.routingRule.allowCutIn ?? true,
    priorityTier: tag?.routingRule.priorityTier ?? 'normal',
    fastModeRewriteMode: tag?.routingRule.fastModeRewriteMode ?? 'keep_original',
    concurrencyLimit: apiConcurrencyLimitToSliderValue(tag?.routingRule.concurrencyLimit),
  }
}

function normalizePositiveInt(value: string): number | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  const parsed = Number(trimmed)
  if (!Number.isInteger(parsed) || parsed <= 0) return null
  return parsed
}

function buildPayload(draft: TagRuleDraft): CreateTagPayload | UpdateTagPayload | null {
  const lookbackHours = normalizePositiveInt(draft.lookbackHours)
  const maxConversations = normalizePositiveInt(draft.maxConversations)
  if (draft.guardEnabled && (lookbackHours == null || maxConversations == null)) return null
  return {
    name: draft.name.trim(),
    guardEnabled: draft.guardEnabled,
    lookbackHours: draft.guardEnabled ? lookbackHours : undefined,
    maxConversations: draft.guardEnabled ? maxConversations : undefined,
    allowCutOut: draft.allowCutOut,
    allowCutIn: draft.allowCutIn,
    priorityTier: draft.priorityTier,
    fastModeRewriteMode: draft.fastModeRewriteMode,
    concurrencyLimit: sliderConcurrencyLimitToApiValue(draft.concurrencyLimit),
  }
}

interface TagRuleDialogProps {
  open: boolean
  mode: TagRuleDialogMode
  tag?: TagSummary | null
  draftName?: string
  busy?: boolean
  error?: string | null
  onClose: () => void
  onSubmit: (payload: CreateTagPayload | UpdateTagPayload) => Promise<void> | void
  labels: {
    createTitle: string
    editTitle: string
    description: string
    name: string
    namePlaceholder: string
    guardEnabled: string
    lookbackHours: string
    maxConversations: string
    allowCutOut: string
    allowCutIn: string
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
    cancel: string
    save: string
    create: string
    validation: string
  }
}

export function TagRuleDialog({ open, mode, tag, draftName, busy = false, error, onClose, onSubmit, labels }: TagRuleDialogProps) {
  const [draft, setDraft] = useState<TagRuleDraft>(() => buildDraft(tag, draftName))

  useEffect(() => {
    if (open) setDraft(buildDraft(tag, draftName))
  }, [draftName, open, tag])

  const payload = useMemo(() => buildPayload(draft), [draft])
  const disabled = !payload || !draft.name.trim() || busy

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy && !nextOpen ? onClose() : undefined)}>
      <DialogContent className="p-0">
        <div className="border-b border-base-300/80 px-6 py-5">
          <DialogHeader>
            <DialogTitle>{mode === 'create' ? labels.createTitle : labels.editTitle}</DialogTitle>
            <DialogDescription>{labels.description}</DialogDescription>
          </DialogHeader>
        </div>
        <div className="space-y-5 px-6 py-5">
          <label className="field">
            <span className="field-label">{labels.name}</span>
            <Input
              name="tagName"
              value={draft.name}
              placeholder={labels.namePlaceholder}
              onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))}
            />
          </label>

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
                <p className="font-medium text-base-content">{labels.guardEnabled}</p>
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
                <p className="font-medium text-base-content">{labels.allowCutOut}</p>
                <Switch checked={draft.allowCutOut} onCheckedChange={(checked) => setDraft((current) => ({ ...current, allowCutOut: checked }))} />
              </div>
            </div>
            <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
              <div className="flex items-center justify-between gap-4">
                <p className="font-medium text-base-content">{labels.allowCutIn}</p>
                <Switch checked={draft.allowCutIn} onCheckedChange={(checked) => setDraft((current) => ({ ...current, allowCutIn: checked }))} />
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
              {mode === 'create' ? labels.create : labels.save}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  )
}
