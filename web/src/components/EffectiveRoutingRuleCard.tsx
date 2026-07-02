import { useEffect, useMemo, useRef, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from './ui/command'
import { Input } from './ui/input'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import { Switch } from './ui/switch'
import type {
  EffectiveRoutingRule,
  EffectiveRoutingTimeoutFieldSources,
  ImageToolRewriteMode,
  PoolRoutingTimeoutSettings,
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
import {
  ROUTING_TIMEOUT_FIELD_ORDER,
  type RoutingTimeoutFieldKey,
} from '../lib/poolRoutingTimeouts'
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
  | 'timeoutResponsesFirstByte'
  | 'timeoutCompactFirstByte'
  | 'timeoutResponsesStream'
  | 'timeoutCompactStream'

type FieldSourceMap = NonNullable<EffectiveRoutingRule['fieldSources']>

const editableFieldSourceKeys: Array<[EditablePolicyField, keyof FieldSourceMap]> = [
  ['allowNewConversations', 'blockNewConversations'],
  ['allowCutOut', 'allowCutOut'],
  ['allowCutIn', 'allowCutIn'],
  ['priorityTier', 'priorityTier'],
  ['fastModeRewriteMode', 'fastModeRewriteMode'],
  ['imageToolRewriteMode', 'imageToolRewriteMode'],
  ['concurrencyLimit', 'concurrencyLimit'],
  ['upstream429Retry', 'upstream429Retry'],
  ['availableModels', 'availableModels'],
]

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
  identityKey?: string | number | null
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
    availableModelsInherited?: string
    availableModelsNoneAllowed?: string
    availableModelsEmpty?: string
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
    timeoutSectionTitle?: string
    timeoutInheritedValue?: string
    timeoutOverrideValue?: string
    timeoutResponsesFirstByte?: string
    timeoutCompactFirstByte?: string
    timeoutResponsesStream?: string
    timeoutCompactStream?: string
    sourceRoot?: string
    sourceGroup?: string
    sourceTag?: string
    sourceAccount?: string
    sourceConversation?: string
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
    upstream429RetryCountValue?: (count: number) => string
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
    timeouts: {
      responsesFirstByteTimeoutSecs: 120,
      compactFirstByteTimeoutSecs: 300,
      responsesStreamTimeoutSecs: 300,
      compactStreamTimeoutSecs: 300,
    },
    timeoutFieldSources: {
      responsesFirstByteTimeoutSecs: 'root',
      compactFirstByteTimeoutSecs: 'root',
      responsesStreamTimeoutSecs: 'root',
      compactStreamTimeoutSecs: 'root',
    },
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
    case 'conversation':
      return labels.sourceConversation ?? 'Conversation'
    case 'system':
      return labels.sourceSystem ?? 'System'
    default:
      return source
  }
}

function sourceVariant(source: string) {
  return source === 'account' || source === 'conversation'
    ? 'default'
    : source === 'tag'
      ? 'accent'
      : source === 'group'
        ? 'info'
        : 'secondary'
}

function accountOverrideFields(fieldSources: FieldSourceMap): EditablePolicyField[] {
  return editableFieldSourceKeys
    .filter(([, sourceKey]) => fieldSources[sourceKey] === 'account')
    .map(([field]) => field)
}

const timeoutFieldToInlineField: Record<RoutingTimeoutFieldKey, EditablePolicyField> = {
  responsesFirstByteTimeoutSecs: 'timeoutResponsesFirstByte',
  compactFirstByteTimeoutSecs: 'timeoutCompactFirstByte',
  responsesStreamTimeoutSecs: 'timeoutResponsesStream',
  compactStreamTimeoutSecs: 'timeoutCompactStream',
}

function accountTimeoutOverrideFields(
  timeoutSources: EffectiveRoutingTimeoutFieldSources,
): EditablePolicyField[] {
  return ROUTING_TIMEOUT_FIELD_ORDER.filter((key) => timeoutSources[key] === 'account').map(
    (key) => timeoutFieldToInlineField[key],
  )
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

function formatUpstream429RetryCount(
  count: number,
  labels: EffectiveRoutingRuleCardProps['labels'],
) {
  const normalized = Math.min(5, Math.max(0, Math.trunc(count)))
  return labels.upstream429RetryCountValue?.(normalized) ?? String(normalized)
}

export function EffectiveRoutingRuleCard({ rule, identityKey, labels, editablePolicy }: EffectiveRoutingRuleCardProps) {
  const resolvedRule = defaultRule(rule)
  const isEditable = editablePolicy != null
  const fieldSources = useMemo(
    () => ({ ...defaultFieldSources, ...(resolvedRule.fieldSources ?? {}) }),
    [resolvedRule.fieldSources],
  )
  const timeoutSources = useMemo<EffectiveRoutingTimeoutFieldSources>(
    () =>
      resolvedRule.timeoutFieldSources ?? {
        responsesFirstByteTimeoutSecs: 'root',
        compactFirstByteTimeoutSecs: 'root',
        responsesStreamTimeoutSecs: 'root',
        compactStreamTimeoutSecs: 'root',
      },
    [resolvedRule.timeoutFieldSources],
  )
  const timeoutValues = useMemo<PoolRoutingTimeoutSettings>(
    () =>
      resolvedRule.timeouts ?? {
        responsesFirstByteTimeoutSecs: 120,
        compactFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
    [resolvedRule.timeouts],
  )
  const defaultExpandedFields = isEditable
    ? [
        ...accountOverrideFields(fieldSources),
        ...accountTimeoutOverrideFields(timeoutSources),
      ]
    : []
  const [expandedFields, setExpandedFields] = useState<EditablePolicyField[]>(defaultExpandedFields)
  const [availableModelInput, setAvailableModelInput] = useState('')
  const userTouchedExpansionRef = useRef(false)
  const previousIdentityKeyRef = useRef(identityKey)

  useEffect(() => {
    const identityChanged = previousIdentityKeyRef.current !== identityKey
    if (identityChanged) {
      previousIdentityKeyRef.current = identityKey
      userTouchedExpansionRef.current = false
    }

    if (!isEditable) {
      userTouchedExpansionRef.current = false
      setExpandedFields([])
      return
    }

    const nextDefaultExpandedFields = [
      ...accountOverrideFields(fieldSources),
      ...accountTimeoutOverrideFields(timeoutSources),
    ]
    setExpandedFields((current) => {
      if (userTouchedExpansionRef.current) return current
      if (current.some((field) => fieldToSource(field, fieldSources, timeoutSources) === 'account')) return current
      return nextDefaultExpandedFields
    })
  }, [
    isEditable,
    identityKey,
    fieldSources.blockNewConversations,
    fieldSources.allowCutOut,
    fieldSources.allowCutIn,
    fieldSources.priorityTier,
    fieldSources.fastModeRewriteMode,
    fieldSources.imageToolRewriteMode,
    fieldSources.concurrencyLimit,
    fieldSources.upstream429Retry,
    fieldSources.availableModels,
    timeoutSources.responsesFirstByteTimeoutSecs,
    timeoutSources.compactFirstByteTimeoutSecs,
    timeoutSources.responsesStreamTimeoutSecs,
    timeoutSources.compactStreamTimeoutSecs,
    fieldSources,
    timeoutSources,
  ])

  const availableModelOptions = useMemo(
    () => normalizeModelIds([...(editablePolicy?.availableModelOptions ?? []), ...(resolvedRule.availableModels ?? [])]),
    [editablePolicy?.availableModelOptions, resolvedRule.availableModels],
  )

  const isBusy = (field: EditablePolicyField) => editablePolicy?.busyField === field
  const changeField = (field: EditablePolicyField, payload: UpdateGroupAccountRoutingRulePayload) => {
    void editablePolicy?.onChange(field, payload)
  }
  const clearField = (
    field: EditablePolicyField,
    payload: UpdateGroupAccountRoutingRulePayload,
  ) => {
    userTouchedExpansionRef.current = true
    changeField(field, payload)
    setExpandedFields((current) => current.filter((value) => value !== field))
  }
  const toggleExpanded = (
    field: EditablePolicyField,
    clearPayload: UpdateGroupAccountRoutingRulePayload,
  ) => {
    userTouchedExpansionRef.current = true
    const active = fieldToSource(field, fieldSources, timeoutSources) === 'account'
    if (active) {
      clearField(field, clearPayload)
      return
    }
    setExpandedFields((current) => (
      current.includes(field)
        ? current.filter((value) => value !== field)
        : [...current, field]
    ))
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
  const upstream429RetryCount =
    resolvedRule.upstream429RetryEnabled === true
      ? Math.min(5, Math.max(0, Math.trunc(resolvedRule.upstream429MaxRetries ?? 0)))
      : 0
  const inlineTimeoutBusy = ROUTING_TIMEOUT_FIELD_ORDER.some(
    (key) => editablePolicy?.busyField === timeoutFieldToInlineField[key],
  )
  const timeoutRows = ROUTING_TIMEOUT_FIELD_ORDER.map((key) => {
    const field = timeoutFieldToInlineField[key]
    const source = timeoutSources[key]
    const label =
      key === 'responsesFirstByteTimeoutSecs'
        ? labels.timeoutResponsesFirstByte ?? 'Standard response first byte timeout'
        : key === 'compactFirstByteTimeoutSecs'
          ? labels.timeoutCompactFirstByte ?? 'Compact response first byte timeout'
          : key === 'responsesStreamTimeoutSecs'
            ? labels.timeoutResponsesStream ?? 'Standard stream completion timeout'
            : labels.timeoutCompactStream ?? 'Compact stream completion timeout'
    return {
      key,
      field,
      label,
      source,
      value: `${timeoutValues[key]}s`,
      clearPayload: {
        timeouts: {
          [key]: null,
        },
      } satisfies UpdateGroupAccountRoutingRulePayload,
    }
  })

  const fieldRows = [
    {
      field: 'allowNewConversations' as const,
      label: labels.newConversationLabel ?? labels.fieldBlockNewConversations ?? 'New conversations',
      value: resolvedRule.blockNewConversations ? labels.blockNewConversations : labels.allowNewConversations,
      source: fieldSources.blockNewConversations,
      clearPayload: { allowNewConversations: null },
      editor: (
        <Switch
          checked={!resolvedRule.blockNewConversations}
          disabled={isBusy('allowNewConversations')}
          onCheckedChange={(checked) => changeField('allowNewConversations', { allowNewConversations: checked })}
          aria-label={labels.newConversationLabel ?? 'New conversations'}
        />
      ),
    },
    {
      field: 'allowCutOut' as const,
      label: labels.cutOutLabel ?? labels.fieldAllowCutOut ?? 'Cut out',
      value: resolvedRule.allowCutOut ? labels.allowCutOut : labels.denyCutOut,
      source: fieldSources.allowCutOut,
      clearPayload: { allowCutOut: null },
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
      clearPayload: { allowCutIn: null },
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
      clearPayload: { priorityTier: null },
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
      clearPayload: { fastModeRewriteMode: null },
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
      clearPayload: { imageToolRewriteMode: null },
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
      clearPayload: { concurrencyLimit: null },
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
      value: formatUpstream429RetryCount(upstream429RetryCount, labels),
      source: fieldSources.upstream429Retry,
      clearPayload: {
        upstream429RetryEnabled: null,
        upstream429MaxRetries: null,
      },
      editor: (
        <RetryInlineEditor
          retries={upstream429RetryCount}
          disabled={isBusy('upstream429Retry')}
          labels={labels}
          onChange={(count) => changeField('upstream429Retry', {
            upstream429RetryEnabled: count > 0,
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
          : fieldSources.availableModels === 'account' ||
              fieldSources.availableModels === 'group' ||
              fieldSources.availableModels === 'tag'
            ? labels.availableModelsNoneAllowed ?? 'No models allowed'
            : labels.availableModelsInherited ?? 'Inherited / unrestricted',
      source: fieldSources.availableModels ?? 'root',
      clearPayload: { availableModels: null },
      editor: (
        <AvailableModelsEditor
          value={availableModelsValue}
          options={availableModelOptions}
          inputValue={availableModelInput}
          emptyValueLabel={
            fieldSources.availableModels === 'account' ||
            fieldSources.availableModels === 'group' ||
            fieldSources.availableModels === 'tag'
              ? labels.availableModelsNoneAllowed ?? 'No models allowed'
              : labels.availableModelsInherited ?? 'Inherited / unrestricted'
          }
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
              const expanded = row.field != null && expandedFields.includes(row.field)
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
                        onClick={() => toggleExpanded(row.field, row.clearPayload)}
                      >
                        <AppIcon name={busy ? 'loading' : activeOverride || expanded ? 'check-decagram-outline' : 'pencil-outline'} className={cn('h-4 w-4', busy ? 'animate-spin' : '')} aria-hidden />
                      </Button>
                  ) : (
                      <span aria-hidden />
                    )}
                  </div>
                  {expanded && row.field ? (
                    <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                      <div className="grid grid-cols-1 gap-y-2 sm:grid-cols-[minmax(7rem,1fr)_minmax(8rem,1.2fr)_minmax(5rem,auto)_2rem] sm:items-center sm:gap-x-3">
                        <p className="text-sm font-semibold text-base-content">{row.label}</p>
                        <div className="min-w-0 sm:col-span-3">{row.editor}</div>
                        {busy ? (
                          <p className="text-xs text-base-content/60 sm:col-start-2 sm:col-span-3">
                            {labels.overrideSaving ?? 'Saving...'}
                          </p>
                        ) : null}
                        {error ? (
                          <p className="text-xs font-medium text-error sm:col-start-2 sm:col-span-3">
                            {error}
                          </p>
                        ) : null}
                      </div>
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
          <p className="metric-label">{labels.timeoutSectionTitle ?? 'Request path timeouts'}</p>
          <div className="mt-3 overflow-hidden rounded-xl border border-base-300/70">
            {timeoutRows.map((row) => {
              const activeOverride = row.source === 'account'
              const expanded = expandedFields.includes(row.field)
              const busy = isBusy(row.field)
              const error = editablePolicy?.errorByField?.[row.field] ?? null
              return (
                <div key={row.key} className="border-b border-base-300/60 last:border-b-0">
                  <div className="grid grid-cols-1 gap-1 px-3 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_5rem_11rem_2rem] sm:items-center sm:gap-3">
                    <span className="min-w-0 font-medium text-base-content/80">{row.label}</span>
                    <span className="whitespace-nowrap text-base-content">{row.value}</span>
                    <div className="min-w-0 flex flex-wrap items-center gap-2">
                      <span className="text-xs text-base-content/65">
                        {activeOverride ? labels.timeoutOverrideValue ?? 'Account override' : labels.timeoutInheritedValue ?? 'Inherited'}
                      </span>
                      <Badge className="w-fit" variant={sourceVariant(row.source)}>
                        {sourceLabel(row.source, labels)}
                      </Badge>
                    </div>
                    {isEditable ? (
                      <Button
                        type="button"
                        size="icon"
                        variant={activeOverride || expanded ? 'default' : 'ghost'}
                        className={cn('h-8 w-8 justify-self-start rounded-full sm:justify-self-end', activeOverride || expanded ? 'text-primary-content' : 'text-base-content/65')}
                        disabled={busy}
                        aria-pressed={activeOverride || expanded}
                        aria-label={`${activeOverride ? labels.overrideClear ?? 'Clear override' : labels.overrideEdit ?? 'Edit override'}: ${row.label}`}
                        onClick={() => toggleExpanded(row.field, row.clearPayload)}
                      >
                        <AppIcon name={busy ? 'loading' : activeOverride || expanded ? 'check-decagram-outline' : 'pencil-outline'} className={cn('h-4 w-4', busy ? 'animate-spin' : '')} aria-hidden />
                      </Button>
                    ) : (
                      <span aria-hidden />
                    )}
                  </div>
                  {expanded ? (
                    <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                      <div className="grid grid-cols-1 gap-y-2 sm:grid-cols-[minmax(0,1fr)_5rem_11rem_2rem] sm:items-center sm:gap-x-3">
                        <p className="min-w-0 text-sm font-semibold text-base-content">{row.label}</p>
                        <div className="min-w-0 sm:col-span-3">
                          <Input
                            name={row.key}
                            type="number"
                            min="1"
                            step="1"
                            defaultValue={String(timeoutValues[row.key])}
                            disabled={busy}
                            className="h-11 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                            onBlur={(event: React.FocusEvent<HTMLInputElement>) => {
                              const parsed = event.currentTarget.value.trim()
                              if (!parsed || !editablePolicy) return
                              void editablePolicy.onChange(row.field, {
                                timeouts: {
                                  [row.key]: Number(parsed),
                                },
                              })
                            }}
                          />
                        </div>
                        {busy ? (
                          <p className="text-xs text-base-content/60 sm:col-start-2 sm:col-span-3">
                            {labels.overrideSaving ?? 'Saving...'}
                          </p>
                        ) : null}
                        {error ? (
                          <p className="text-xs font-medium text-error sm:col-start-2 sm:col-span-3">
                            {error}
                          </p>
                        ) : null}
                      </div>
                    </div>
                  ) : error ? (
                    <p className="px-3 pb-2 text-xs font-medium text-error">{error}</p>
                  ) : null}
                </div>
              )
            })}
          </div>
          {inlineTimeoutBusy ? (
            <p className="mt-3 text-xs text-base-content/60">
              {labels.overrideSaving ?? 'Saving...'}
            </p>
          ) : null}
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

function fieldToSource(
  field: EditablePolicyField,
  sources: FieldSourceMap,
  timeoutSources: EffectiveRoutingTimeoutFieldSources,
): string {
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
    case 'timeoutResponsesFirstByte':
      return timeoutSources.responsesFirstByteTimeoutSecs
    case 'timeoutCompactFirstByte':
      return timeoutSources.compactFirstByteTimeoutSecs
    case 'timeoutResponsesStream':
      return timeoutSources.responsesStreamTimeoutSecs
    case 'timeoutCompactStream':
      return timeoutSources.compactStreamTimeoutSecs
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
  retries: number
  disabled?: boolean
  labels: EffectiveRoutingRuleCardProps['labels']
  onChange: (count: number) => void
}

function RetryInlineEditor({ retries, disabled, labels, onChange }: RetryInlineEditorProps) {
  const value = Math.min(5, Math.max(0, Math.trunc(retries)))
  return (
    <InlineOptionGroup<number>
      ariaLabel={labels.fieldUpstream429 ?? 'Upstream 429 retry'}
      value={value}
      disabled={disabled}
      options={[0, 1, 2, 3, 4, 5].map((option) => ({
        value: option,
        label: formatUpstream429RetryCount(option, labels),
      }))}
      onChange={onChange}
    />
  )
}

interface AvailableModelsEditorProps {
  value: string[]
  options: string[]
  inputValue: string
  emptyValueLabel: string
  disabled?: boolean
  labels: EffectiveRoutingRuleCardProps['labels']
  onInputChange: (value: string) => void
  onAdd: (value: string) => void
  onChange: (value: string[]) => void
}

function AvailableModelsEditor({
  value,
  options,
  inputValue,
  emptyValueLabel,
  disabled,
  labels,
  onInputChange,
  onAdd,
  onChange,
}: AvailableModelsEditorProps) {
  const trimmedInput = inputValue.trim()
  const canAdd = trimmedInput.length > 0 && !value.includes(trimmedInput)
  const [open, setOpen] = useState(false)
  const selectedValueSet = useMemo(() => new Set(value), [value])
  const availableOptions = useMemo(
    () => options.filter((option, index) => option.trim() && options.indexOf(option) === index),
    [options],
  )
  const filteredOptions = useMemo(() => {
    if (!trimmedInput) return availableOptions
    const query = trimmedInput.toLocaleLowerCase()
    return availableOptions.filter((option) => option.toLocaleLowerCase().includes(query))
  }, [availableOptions, trimmedInput])

  const commitCustomValue = () => {
    if (!canAdd) return
    onAdd(trimmedInput)
    setOpen(false)
  }

  const triggerTitle = value.length > 0 ? value.join(', ') : emptyValueLabel

  return (
    <div className="min-w-[18rem]">
      <Popover
        open={disabled ? false : open}
        onOpenChange={(nextOpen) => {
          if (disabled) {
            setOpen(false)
            return
          }
          setOpen(nextOpen)
          if (!nextOpen) {
            onInputChange('')
          }
        }}
      >
        <PopoverTrigger asChild>
          <button
            type="button"
            role="combobox"
            aria-expanded={open}
            aria-label={labels.fieldAvailableModels ?? 'Available models'}
            disabled={disabled}
            title={triggerTitle}
            className={cn(
              'flex w-full items-center gap-3 rounded-xl border border-base-300 bg-base-100 px-3 py-2.5 text-left shadow-sm transition-colors',
              'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100',
              'hover:border-primary/35',
              disabled && 'cursor-not-allowed opacity-60',
            )}
          >
            <AppIcon name="tag-outline" className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55" aria-hidden />
            <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
              {value.length > 0 ? (
                value.map((model) => (
                  <Badge key={model} variant="secondary" className="max-w-full rounded-full border border-primary/20 bg-primary/10 px-2.5 py-1 text-primary">
                    <span className="truncate">{labels.availableModelsCustomLabel?.(model) ?? model}</span>
                  </Badge>
                ))
              ) : (
                <span className="text-sm text-base-content/55">{emptyValueLabel}</span>
              )}
            </span>
            <AppIcon name="chevron-down" className="h-4 w-4 shrink-0 text-base-content/45" aria-hidden />
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-[var(--radix-popover-trigger-width)] p-0">
          <Command shouldFilter={false}>
            <CommandInput
              value={inputValue}
              placeholder={labels.availableModelsPlaceholder ?? labels.availableModelsAddCustom ?? 'Add model'}
              onValueChange={onInputChange}
            />
            <CommandList>
              {canAdd ? (
                <>
                  <CommandGroup>
                    <CommandItem value={trimmedInput} onSelect={commitCustomValue}>
                      <AppIcon name="plus-circle-outline" className="mr-2 h-4 w-4 text-primary" aria-hidden />
                      <span className="truncate">{trimmedInput}</span>
                    </CommandItem>
                  </CommandGroup>
                  <CommandSeparator />
                </>
              ) : null}
              {filteredOptions.length === 0 ? (
                <CommandEmpty>{labels.availableModelsEmpty ?? 'No matching models'}</CommandEmpty>
              ) : (
                <CommandGroup>
                  {filteredOptions.map((model) => {
                    const active = selectedValueSet.has(model)
                    return (
                      <CommandItem
                        key={model}
                        value={model}
                        disabled={disabled}
                        onSelect={() => onChange(active ? value.filter((item) => item !== model) : [...value, model])}
                      >
                        <AppIcon
                          name="check"
                          className={cn(
                            'mr-2 h-4 w-4 text-primary transition-opacity',
                            active ? 'opacity-100' : 'opacity-0',
                          )}
                          aria-hidden
                        />
                        <span className="truncate">{labels.availableModelsCustomLabel?.(model) ?? model}</span>
                      </CommandItem>
                    )
                  })}
                </CommandGroup>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  )
}
