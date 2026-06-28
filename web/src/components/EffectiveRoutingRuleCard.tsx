import { useMemo, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import { Input } from './ui/input'
import { Switch } from './ui/switch'
import type {
  EffectiveRoutingRule,
  ImageToolRewriteMode,
  TagFastModeRewriteMode,
  TagPriorityTier,
  UpdateGroupAccountRoutingRulePayload,
} from '../lib/api'
import {
  CONCURRENCY_LIMIT_MAX,
  CONCURRENCY_LIMIT_MIN,
  CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE,
  apiConcurrencyLimitToSliderValue,
  formatConcurrencyLimitValue,
  sliderConcurrencyLimitToApiValue,
} from '../lib/concurrencyLimit'
import {
  fastModeRewriteBadgeLabel,
  priorityTierBadgeLabel,
} from '../lib/tagRoutingRule'
import { cn } from '../lib/utils'

type EditablePolicyField =
  | 'allowNewConversations'
  | 'allowCutOut'
  | 'allowCutIn'
  | 'priorityTier'
  | 'fastModeRewriteMode'
  | 'imageToolRewriteMode'
  | 'concurrencyLimit'
  | 'upstream429Retry'
  | 'availableModels'

type FieldSourceMap = NonNullable<EffectiveRoutingRule['fieldSources']>

interface InlineOption<T extends string | number> {
  value: T
  label: string
}

interface InlineOptionGroupProps<T extends string | number> {
  ariaLabel: string
  value: T
  options: InlineOption<T>[]
  disabled?: boolean
  onChange: (value: T) => void
}

function InlineOptionGroup<T extends string | number>({
  ariaLabel,
  value,
  options,
  disabled,
  onChange,
}: InlineOptionGroupProps<T>) {
  const activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  )
  return (
    <div
      className="policy-inline-radio"
      role="radiogroup"
      aria-label={ariaLabel}
      style={{ ['--option-count' as string]: options.length, ['--active-index' as string]: activeIndex }}
    >
      <span className="policy-inline-radio-indicator" aria-hidden />
      {options.map((option) => (
        <button
          key={String(option.value)}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          disabled={disabled}
          className="policy-inline-radio-item"
          data-active={option.value === value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  )
}

interface EditablePolicyConfig {
  busyField?: EditablePolicyField | null
  errorByField?: Partial<Record<EditablePolicyField, string | null>>
  availableModelOptions?: string[]
  onChange: (
    field: EditablePolicyField,
    payload: UpdateGroupAccountRoutingRulePayload,
  ) => Promise<void> | void
}

interface EffectiveRoutingRuleCardProps {
  rule?: EffectiveRoutingRule | null
  editablePolicy?: EditablePolicyConfig
  labels: {
    title: string
    description: string
    noTags: string
    blockNewConversations: string
    allowNewConversations: string
    allowCutOut: string
    denyCutOut: string
    allowCutIn: string
    denyCutIn: string
    sourceTags: string
    priorityPrimary: string
    priorityNormal: string
    priorityFallback: string
    fastModeKeepOriginal: string
    fastModeFillMissing: string
    fastModeForceAdd: string
    fastModeForceRemove: string
    imageToolKeepOriginal: string
    imageToolFillMissing: string
    imageToolForceAdd: string
    imageToolForceRemove: string
    upstream429Retry?: string
    upstream429RetryOff?: string
    availableModelsInherited?: string
    availableModelsNoneAllowed?: string
    availableModelsField?: string
    systemDeniedModelsField?: string
    systemDeniedModelsEmpty?: string
    concurrencyLimit?: (count: number) => string
    concurrencyUnlimited?: string
    sourceBreakdownTitle?: string
    fieldBlockNewConversations?: string
    fieldAllowCutOut?: string
    fieldAllowCutIn?: string
    fieldPriority?: string
    fieldFastMode?: string
    fieldImageToolRewriteMode?: string
    fieldConcurrency?: string
    fieldUpstream429?: string
    fieldAvailableModels?: string
    fieldSystemDeniedModels?: string
    sourceRoot?: string
    sourceGroup?: string
    sourceTag?: string
    sourceAccount?: string
    sourceSystem?: string
    overrideEdit?: string
    overrideActive?: string
    overrideClear?: string
    overrideSaving?: string
    inheritValue?: string
    allowLabel?: string
    denyLabel?: string
    newConversationLabel?: string
    cutOutLabel?: string
    cutInLabel?: string
    upstream429RetryCountOnce?: string
    upstream429RetryCountMany?: (count: number) => string
    availableModelsAddCustom?: string
    availableModelsCustomLabel?: (value: string) => string
    availableModelsRemove?: string
    availableModelsPlaceholder?: string
    currentValue?: string
  }
}

const defaultFieldSources: FieldSourceMap = {
  blockNewConversations: 'root',
  allowCutOut: 'root',
  allowCutIn: 'root',
  priorityTier: 'root',
  fastModeRewriteMode: 'root',
  imageToolRewriteMode: 'root',
  concurrencyLimit: 'root',
  upstream429Retry: 'root',
  availableModels: 'root',
  systemDeniedModels: 'root',
}

function defaultRule(rule?: EffectiveRoutingRule | null): EffectiveRoutingRule {
  return rule ?? {
    blockNewConversations: false,
    allowCutOut: true,
    allowCutIn: true,
    priorityTier: 'normal',
    fastModeRewriteMode: 'keep_original',
    imageToolRewriteMode: 'keep_original',
    sourceTagIds: [],
    sourceTagNames: [],
    concurrencyLimit: 0,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    availableModels: [],
    systemDeniedModels: [],
    fieldSources: defaultFieldSources,
  }
}

function sourceLabel(source: string, labels: EffectiveRoutingRuleCardProps['labels']): string {
  switch (source) {
    case 'root':
      return labels.sourceRoot ?? 'Root default'
    case 'group':
      return labels.sourceGroup ?? 'Group'
    case 'tag':
      return labels.sourceTag ?? 'Tag'
    case 'account':
      return labels.sourceAccount ?? 'Account'
    case 'system':
      return labels.sourceSystem ?? 'System'
    default:
      return source
  }
}

function sourceVariant(source: string) {
  return source === 'account' ? 'default' : source === 'tag' ? 'accent' : source === 'group' ? 'info' : 'secondary'
}

function normalizeModelIds(values: string[]) {
  const seen = new Set<string>()
  const normalized: string[] = []
  for (const value of values) {
    const trimmed = value.trim()
    if (!trimmed || seen.has(trimmed)) continue
    seen.add(trimmed)
    normalized.push(trimmed)
  }
  return normalized
}

export function EffectiveRoutingRuleCard({ rule, labels, editablePolicy }: EffectiveRoutingRuleCardProps) {
  const resolvedRule = defaultRule(rule)
  const fieldSources = { ...defaultFieldSources, ...(resolvedRule.fieldSources ?? {}) }
  const [expandedField, setExpandedField] = useState<EditablePolicyField | null>(null)
  const [availableModelInput, setAvailableModelInput] = useState('')

  const availableModelOptions = useMemo(
    () => normalizeModelIds([...(editablePolicy?.availableModelOptions ?? []), ...(resolvedRule.availableModels ?? [])]),
    [editablePolicy?.availableModelOptions, resolvedRule.availableModels],
  )

  const isBusy = (field: EditablePolicyField) => editablePolicy?.busyField === field
  const changeField = (field: EditablePolicyField, payload: UpdateGroupAccountRoutingRulePayload) => {
    void editablePolicy?.onChange(field, payload)
  }
  const clearField = (field: EditablePolicyField, payloadKey: keyof UpdateGroupAccountRoutingRulePayload) => {
    changeField(field, { [payloadKey]: null } as UpdateGroupAccountRoutingRulePayload)
    setExpandedField((current) => (current === field ? null : current))
  }
  const toggleExpanded = (field: EditablePolicyField, payloadKey: keyof UpdateGroupAccountRoutingRulePayload) => {
    const active = fieldToSource(field, fieldSources) === 'account'
    if (active) {
      clearField(field, payloadKey)
      return
    }
    setExpandedField((current) => (current === field ? null : field))
  }

  const availableModelsValue = normalizeModelIds(resolvedRule.availableModels ?? [])
  const updateAvailableModels = (nextModels: string[]) => {
    changeField('availableModels', { availableModels: normalizeModelIds(nextModels) })
  }
  const appendAvailableModel = (model: string) => {
    const trimmed = model.trim()
    if (!trimmed || availableModelsValue.includes(trimmed)) return
    updateAvailableModels([...availableModelsValue, trimmed])
    setAvailableModelInput('')
  }

  const fieldRows = [
    {
      field: 'allowNewConversations' as const,
      label: labels.newConversationLabel ?? labels.fieldBlockNewConversations ?? 'New conversations',
      value: resolvedRule.blockNewConversations ? labels.blockNewConversations : labels.allowNewConversations,
      source: fieldSources.blockNewConversations,
      payloadKey: 'blockNewConversations' as const,
      editor: (
        <Switch
          checked={!resolvedRule.blockNewConversations}
          disabled={isBusy('allowNewConversations')}
          onCheckedChange={(checked) => changeField('allowNewConversations', { blockNewConversations: !checked })}
          aria-label={labels.newConversationLabel ?? 'New conversations'}
        />
      ),
    },
    {
      field: 'allowCutOut' as const,
      label: labels.cutOutLabel ?? labels.fieldAllowCutOut ?? 'Cut out',
      value: resolvedRule.allowCutOut ? labels.allowCutOut : labels.denyCutOut,
      source: fieldSources.allowCutOut,
      payloadKey: 'allowCutOut' as const,
      editor: (
        <Switch
          checked={resolvedRule.allowCutOut}
          disabled={isBusy('allowCutOut')}
          onCheckedChange={(checked) => changeField('allowCutOut', { allowCutOut: checked })}
          aria-label={labels.cutOutLabel ?? 'Cut out'}
        />
      ),
    },
    {
      field: 'allowCutIn' as const,
      label: labels.cutInLabel ?? labels.fieldAllowCutIn ?? 'Cut in',
      value: resolvedRule.allowCutIn ? labels.allowCutIn : labels.denyCutIn,
      source: fieldSources.allowCutIn,
      payloadKey: 'allowCutIn' as const,
      editor: (
        <Switch
          checked={resolvedRule.allowCutIn}
          disabled={isBusy('allowCutIn')}
          onCheckedChange={(checked) => changeField('allowCutIn', { allowCutIn: checked })}
          aria-label={labels.cutInLabel ?? 'Cut in'}
        />
      ),
    },
    {
      field: 'priorityTier' as const,
      label: labels.fieldPriority ?? 'Priority',
      value: priorityTierBadgeLabel(resolvedRule.priorityTier, labels),
      source: fieldSources.priorityTier,
      payloadKey: 'priorityTier' as const,
      editor: (
        <InlineOptionGroup<TagPriorityTier>
          ariaLabel={labels.fieldPriority ?? 'Priority'}
          value={resolvedRule.priorityTier ?? 'normal'}
          disabled={isBusy('priorityTier')}
          options={[
            { value: 'primary', label: labels.priorityPrimary },
            { value: 'normal', label: labels.priorityNormal },
            { value: 'fallback', label: labels.priorityFallback },
          ]}
          onChange={(value) => changeField('priorityTier', { priorityTier: value })}
        />
      ),
    },
    {
      field: 'fastModeRewriteMode' as const,
      label: labels.fieldFastMode ?? 'FAST mode',
      value: fastModeRewriteBadgeLabel(resolvedRule.fastModeRewriteMode, labels),
      source: fieldSources.fastModeRewriteMode,
      payloadKey: 'fastModeRewriteMode' as const,
      editor: (
        <InlineOptionGroup<TagFastModeRewriteMode>
          ariaLabel={labels.fieldFastMode ?? 'FAST mode'}
          value={resolvedRule.fastModeRewriteMode ?? 'keep_original'}
          disabled={isBusy('fastModeRewriteMode')}
          options={[
            { value: 'keep_original', label: labels.fastModeKeepOriginal },
            { value: 'fill_missing', label: labels.fastModeFillMissing },
            { value: 'force_add', label: labels.fastModeForceAdd },
            { value: 'force_remove', label: labels.fastModeForceRemove },
          ]}
          onChange={(value) => changeField('fastModeRewriteMode', { fastModeRewriteMode: value })}
        />
      ),
    },
    {
      field: 'imageToolRewriteMode' as const,
      label: labels.fieldImageToolRewriteMode ?? 'Image tools',
      value:
        resolvedRule.imageToolRewriteMode === 'fill_missing'
          ? labels.imageToolFillMissing
          : resolvedRule.imageToolRewriteMode === 'force_add'
            ? labels.imageToolForceAdd
            : resolvedRule.imageToolRewriteMode === 'force_remove'
              ? labels.imageToolForceRemove
              : labels.imageToolKeepOriginal,
      source: fieldSources.imageToolRewriteMode ?? 'root',
      payloadKey: 'imageToolRewriteMode' as const,
      editor: (
        <InlineOptionGroup<ImageToolRewriteMode>
          ariaLabel={labels.fieldImageToolRewriteMode ?? 'Image tools'}
          value={resolvedRule.imageToolRewriteMode ?? 'keep_original'}
          disabled={isBusy('imageToolRewriteMode')}
          options={[
            { value: 'keep_original', label: labels.imageToolKeepOriginal },
            { value: 'fill_missing', label: labels.imageToolFillMissing },
            { value: 'force_add', label: labels.imageToolForceAdd },
            { value: 'force_remove', label: labels.imageToolForceRemove },
          ]}
          onChange={(value) => changeField('imageToolRewriteMode', { imageToolRewriteMode: value })}
        />
      ),
    },
    {
      field: 'concurrencyLimit' as const,
      label: labels.fieldConcurrency ?? 'Concurrency',
      value: resolvedRule.concurrencyLimit
        ? labels.concurrencyLimit?.(resolvedRule.concurrencyLimit) ?? `Concurrency ${resolvedRule.concurrencyLimit}`
        : labels.concurrencyUnlimited ?? 'Concurrency unlimited',
      source: fieldSources.concurrencyLimit,
      payloadKey: 'concurrencyLimit' as const,
      editor: (
        <ConcurrencyInlineEditor
          value={resolvedRule.concurrencyLimit ?? 0}
          disabled={isBusy('concurrencyLimit')}
          currentLabel={labels.currentValue ?? 'Current'}
          unlimitedLabel={labels.concurrencyUnlimited ?? 'Unlimited'}
          onChange={(value) => changeField('concurrencyLimit', { concurrencyLimit: value })}
        />
      ),
    },
    {
      field: 'upstream429Retry' as const,
      label: labels.fieldUpstream429 ?? 'Upstream 429 retry',
      value: resolvedRule.upstream429RetryEnabled
        ? labels.upstream429Retry ?? `429 retry x${resolvedRule.upstream429MaxRetries ?? 1}`
        : labels.upstream429RetryOff ?? '429 retry off',
      source: fieldSources.upstream429Retry,
      payloadKey: 'upstream429RetryEnabled' as const,
      editor: (
        <RetryInlineEditor
          enabled={resolvedRule.upstream429RetryEnabled === true}
          retries={resolvedRule.upstream429MaxRetries ?? 0}
          disabled={isBusy('upstream429Retry')}
          labels={labels}
          onEnabledChange={(checked) => changeField('upstream429Retry', {
            upstream429RetryEnabled: checked,
            upstream429MaxRetries: checked ? Math.max(1, resolvedRule.upstream429MaxRetries || 1) : 0,
          })}
          onRetriesChange={(count) => changeField('upstream429Retry', {
            upstream429RetryEnabled: true,
            upstream429MaxRetries: count,
          })}
        />
      ),
    },
    {
      field: 'availableModels' as const,
      label: labels.fieldAvailableModels ?? 'Available models',
      value:
        availableModelsValue.length > 0
          ? availableModelsValue.join(', ')
          : fieldSources.availableModels === 'account' || fieldSources.availableModels === 'tag'
            ? labels.availableModelsNoneAllowed ?? 'No models allowed'
            : labels.availableModelsInherited ?? 'Inherited / unrestricted',
      source: fieldSources.availableModels ?? 'root',
      payloadKey: 'availableModels' as const,
      editor: (
        <AvailableModelsEditor
          value={availableModelsValue}
          options={availableModelOptions}
          inputValue={availableModelInput}
          disabled={isBusy('availableModels')}
          labels={labels}
          onInputChange={setAvailableModelInput}
          onAdd={appendAvailableModel}
          onChange={updateAvailableModels}
        />
      ),
    },
    {
      field: null,
      label: labels.fieldSystemDeniedModels ?? 'System denied models',
      value: resolvedRule.systemDeniedModels && resolvedRule.systemDeniedModels.length > 0
        ? resolvedRule.systemDeniedModels.join(', ')
        : labels.systemDeniedModelsEmpty ?? 'None',
      source: fieldSources.systemDeniedModels ?? 'root',
    },
  ]
  const blockingBadges = [
    resolvedRule.blockNewConversations ? labels.blockNewConversations : null,
    !resolvedRule.allowCutOut ? labels.denyCutOut : null,
    !resolvedRule.allowCutIn ? labels.denyCutIn : null,
  ].filter((value): value is string => value != null)

  return (
    <Card className="border-base-300/80 bg-base-100/72">
      <CardHeader>
        <CardTitle>{labels.title}</CardTitle>
        <CardDescription>{labels.description}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {blockingBadges.length > 0 ? (
          <div className="flex flex-wrap gap-2">
            {blockingBadges.map((label) => (
              <Badge key={label} variant="warning">
                {label}
              </Badge>
            ))}
          </div>
        ) : null}

        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-3">
          <p className="metric-label">{labels.sourceBreakdownTitle ?? 'Field source breakdown'}</p>
          <div className="mt-3 overflow-hidden rounded-xl border border-base-300/70">
            {fieldRows.map((row) => {
              const editable = row.field != null && editablePolicy != null
              const activeOverride = row.field != null && row.source === 'account'
              const expanded = row.field != null && expandedField === row.field
              const error = row.field != null ? editablePolicy?.errorByField?.[row.field] : null
              const busy = row.field != null && isBusy(row.field)
              return (
                <div key={row.label} className="border-b border-base-300/60 last:border-b-0">
                  <div className="grid grid-cols-1 gap-1 px-3 py-2.5 text-sm sm:grid-cols-[minmax(7rem,1fr)_minmax(8rem,1.2fr)_minmax(5rem,auto)_2rem] sm:items-center sm:gap-3">
                    <span className="font-medium text-base-content/80">{row.label}</span>
                    <span className="text-base-content">{row.value}</span>
                    <Badge className="w-fit sm:justify-self-end" variant={sourceVariant(row.source)}>
                      {sourceLabel(row.source, labels)}
                    </Badge>
                    {editable && row.field ? (
                      <Button
                        type="button"
                        size="icon"
                        variant={activeOverride || expanded ? 'default' : 'ghost'}
                        className={cn('h-8 w-8 justify-self-start rounded-full sm:justify-self-end', activeOverride || expanded ? 'text-primary-content' : 'text-base-content/65')}
                        disabled={busy}
                        aria-pressed={activeOverride || expanded}
                        aria-label={`${activeOverride ? labels.overrideClear ?? 'Clear override' : labels.overrideEdit ?? 'Edit override'}: ${row.label}`}
                        onClick={() => toggleExpanded(row.field, row.payloadKey)}
                      >
                        <AppIcon name={busy ? 'loading' : activeOverride || expanded ? 'check-decagram-outline' : 'pencil-outline'} className={cn('h-4 w-4', busy ? 'animate-spin' : '')} aria-hidden />
                      </Button>
                    ) : (
                      <span aria-hidden />
                    )}
                  </div>
                  {expanded && row.field ? (
                    <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                      <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                        <div className="space-y-1">
                          <p className="text-sm font-semibold text-base-content">
                            {labels.overrideActive ?? 'Account override'}
                          </p>
                          <p className="text-xs leading-5 text-base-content/65">
                            {labels.inheritValue ?? 'Default value is the inherited value.'}
                          </p>
                        </div>
                        <div className="min-w-0 md:max-w-[min(34rem,100%)]">{row.editor}</div>
                      </div>
                      {busy ? <p className="mt-2 text-xs text-base-content/60">{labels.overrideSaving ?? 'Saving...'}</p> : null}
                      {error ? <p className="mt-2 text-xs font-medium text-error">{error}</p> : null}
                    </div>
                  ) : error ? (
                    <p className="px-3 pb-2 text-xs font-medium text-error">{error}</p>
                  ) : null}
                </div>
              )
            })}
          </div>
        </div>

        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-3">
          <p className="metric-label">{labels.sourceTags}</p>
          <div className="mt-3 flex flex-wrap gap-2">
            {resolvedRule.sourceTagNames.length === 0 ? (
              <span className="text-sm text-base-content/60">{labels.noTags}</span>
            ) : (
              resolvedRule.sourceTagNames.map((name) => (
                <Badge key={name} variant="secondary">{name}</Badge>
              ))
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function fieldToSource(field: EditablePolicyField, sources: FieldSourceMap): string {
  switch (field) {
    case 'allowNewConversations':
      return sources.blockNewConversations
    case 'allowCutOut':
      return sources.allowCutOut
    case 'allowCutIn':
      return sources.allowCutIn
    case 'priorityTier':
      return sources.priorityTier
    case 'fastModeRewriteMode':
      return sources.fastModeRewriteMode
    case 'imageToolRewriteMode':
      return sources.imageToolRewriteMode ?? 'root'
    case 'concurrencyLimit':
      return sources.concurrencyLimit
    case 'upstream429Retry':
      return sources.upstream429Retry
    case 'availableModels':
      return sources.availableModels ?? 'root'
  }
}

interface ConcurrencyInlineEditorProps {
  value: number
  disabled?: boolean
  currentLabel: string
  unlimitedLabel: string
  onChange: (value: number) => void
}

function ConcurrencyInlineEditor({ value, disabled, currentLabel, unlimitedLabel, onChange }: ConcurrencyInlineEditorProps) {
  const sliderValue = apiConcurrencyLimitToSliderValue(value)
  const displayValue = formatConcurrencyLimitValue(value, unlimitedLabel)
  return (
    <div className="min-w-[16rem] space-y-2">
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs font-semibold uppercase tracking-[0.12em] text-base-content/55">{currentLabel}</span>
        <span className="rounded-full border border-base-300/80 bg-base-200/80 px-2.5 py-1 text-sm font-semibold text-base-content">{displayValue}</span>
      </div>
      <input
        type="range"
        min={CONCURRENCY_LIMIT_MIN}
        max={CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE}
        step={1}
        value={sliderValue}
        disabled={disabled}
        aria-label={currentLabel}
        aria-valuetext={displayValue}
        onChange={(event) => onChange(sliderConcurrencyLimitToApiValue(Number(event.target.value)))}
        className="h-2 w-full cursor-pointer appearance-none rounded-full bg-base-300 accent-primary disabled:cursor-not-allowed disabled:opacity-60"
      />
      <div className="flex items-center justify-between text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/45">
        <span>{CONCURRENCY_LIMIT_MIN}</span>
        <span>{CONCURRENCY_LIMIT_MAX}</span>
        <span title={unlimitedLabel}>∞</span>
      </div>
    </div>
  )
}

interface RetryInlineEditorProps {
  enabled: boolean
  retries: number
  disabled?: boolean
  labels: EffectiveRoutingRuleCardProps['labels']
  onEnabledChange: (checked: boolean) => void
  onRetriesChange: (count: number) => void
}

function RetryInlineEditor({ enabled, retries, disabled, labels, onEnabledChange, onRetriesChange }: RetryInlineEditorProps) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
      <Switch checked={enabled} disabled={disabled} onCheckedChange={onEnabledChange} aria-label={labels.fieldUpstream429 ?? 'Upstream 429 retry'} />
      <InlineOptionGroup<number>
        ariaLabel={labels.fieldUpstream429 ?? 'Upstream 429 retry'}
        value={Math.max(1, retries || 1)}
        disabled={disabled || !enabled}
        options={[1, 2, 3, 4, 5].map((value) => ({
          value,
          label:
            value === 1
              ? labels.upstream429RetryCountOnce ?? '1 retry'
              : labels.upstream429RetryCountMany?.(value) ?? `${value} retries`,
        }))}
        onChange={onRetriesChange}
      />
    </div>
  )
}

interface AvailableModelsEditorProps {
  value: string[]
  options: string[]
  inputValue: string
  disabled?: boolean
  labels: EffectiveRoutingRuleCardProps['labels']
  onInputChange: (value: string) => void
  onAdd: (value: string) => void
  onChange: (value: string[]) => void
}

function AvailableModelsEditor({ value, options, inputValue, disabled, labels, onInputChange, onAdd, onChange }: AvailableModelsEditorProps) {
  const trimmedInput = inputValue.trim()
  const canAdd = trimmedInput.length > 0 && !value.includes(trimmedInput)
  return (
    <div className="min-w-[18rem] space-y-3">
      {options.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {options.map((model) => {
            const active = value.includes(model)
            return (
              <button
                key={model}
                type="button"
                disabled={disabled}
                data-active={active}
                className="rounded-full border border-base-300/80 bg-base-100/80 px-3 py-1.5 text-sm font-medium text-base-content/70 transition hover:border-primary/45 hover:text-primary data-[active=true]:border-primary/45 data-[active=true]:bg-primary/15 data-[active=true]:text-primary disabled:cursor-not-allowed disabled:opacity-60"
                onClick={() => onChange(active ? value.filter((item) => item !== model) : [...value, model])}
              >
                {labels.availableModelsCustomLabel?.(model) ?? model}
              </button>
            )
          })}
        </div>
      ) : null}
      <div className="flex gap-2">
        <Input
          value={inputValue}
          disabled={disabled}
          placeholder={labels.availableModelsPlaceholder ?? labels.availableModelsAddCustom ?? 'Add model'}
          onChange={(event) => onInputChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || !canAdd) return
            event.preventDefault()
            onAdd(trimmedInput)
          }}
        />
        <Button type="button" variant="outline" disabled={disabled || !canAdd} onClick={() => onAdd(trimmedInput)}>
          <AppIcon name="plus" className="mr-2 h-4 w-4" aria-hidden />
          {labels.availableModelsAddCustom ?? 'Add'}
        </Button>
      </div>
      {value.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {value.map((model) => (
            <Badge key={model} variant="secondary" className="gap-1 pr-1">
              <span>{labels.availableModelsCustomLabel?.(model) ?? model}</span>
              <button
                type="button"
                disabled={disabled}
                className="rounded-full p-0.5 text-base-content/55 transition hover:bg-base-300/70 hover:text-base-content disabled:cursor-not-allowed"
                aria-label={`${labels.availableModelsRemove ?? 'Remove'} ${model}`}
                onClick={() => onChange(value.filter((item) => item !== model))}
              >
                <AppIcon name="close" className="h-3 w-3" aria-hidden />
              </button>
            </Badge>
          ))}
        </div>
      ) : (
        <p className="text-xs leading-5 text-base-content/60">{labels.availableModelsNoneAllowed ?? 'No models allowed'}</p>
      )}
    </div>
  )
}
