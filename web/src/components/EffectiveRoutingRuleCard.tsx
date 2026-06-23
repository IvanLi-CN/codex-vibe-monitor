import { Badge } from './ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import type { EffectiveRoutingRule } from '../lib/api'
import {
  fastModeRewriteBadgeLabel,
  priorityTierBadgeLabel,
} from '../lib/tagRoutingRule'

interface EffectiveRoutingRuleCardProps {
  rule?: EffectiveRoutingRule | null
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
  }
}

export function EffectiveRoutingRuleCard({ rule, labels }: EffectiveRoutingRuleCardProps) {
  const resolvedRule: EffectiveRoutingRule = rule ?? {
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
    fieldSources: {
      blockNewConversations: 'root',
      allowCutOut: 'root',
      allowCutIn: 'root',
      priorityTier: 'root',
      fastModeRewriteMode: 'root',
      imageToolRewriteMode: 'root',
      concurrencyLimit: 'root',
      upstream429Retry: 'root',
    },
  }
  const fieldSources = resolvedRule.fieldSources ?? {
    blockNewConversations: 'root',
    allowCutOut: 'root',
    allowCutIn: 'root',
      priorityTier: 'root',
      fastModeRewriteMode: 'root',
      imageToolRewriteMode: 'root',
      concurrencyLimit: 'root',
      upstream429Retry: 'root',
    }
  const sourceLabel = (source: string): string => {
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
  const fieldRows = [
    {
      label: labels.fieldBlockNewConversations ?? 'Block new conversations',
      value: resolvedRule.blockNewConversations ? labels.blockNewConversations : labels.allowNewConversations,
      source: fieldSources.blockNewConversations,
    },
    {
      label: labels.fieldAllowCutOut ?? 'Cut out',
      value: resolvedRule.allowCutOut ? labels.allowCutOut : labels.denyCutOut,
      source: fieldSources.allowCutOut,
    },
    {
      label: labels.fieldAllowCutIn ?? 'Cut in',
      value: resolvedRule.allowCutIn ? labels.allowCutIn : labels.denyCutIn,
      source: fieldSources.allowCutIn,
    },
    {
      label: labels.fieldPriority ?? 'Priority',
      value: priorityTierBadgeLabel(resolvedRule.priorityTier, labels),
      source: fieldSources.priorityTier,
    },
    {
      label: labels.fieldFastMode ?? 'FAST mode',
      value: fastModeRewriteBadgeLabel(resolvedRule.fastModeRewriteMode, labels),
      source: fieldSources.fastModeRewriteMode,
    },
    {
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
    },
    {
      label: labels.fieldConcurrency ?? 'Concurrency',
      value: resolvedRule.concurrencyLimit
        ? labels.concurrencyLimit?.(resolvedRule.concurrencyLimit) ?? `Concurrency ${resolvedRule.concurrencyLimit}`
        : labels.concurrencyUnlimited ?? 'Concurrency unlimited',
      source: fieldSources.concurrencyLimit,
    },
    {
      label: labels.fieldUpstream429 ?? 'Upstream 429 retry',
      value: resolvedRule.upstream429RetryEnabled
        ? labels.upstream429Retry ?? `429 retry x${resolvedRule.upstream429MaxRetries ?? 1}`
        : labels.upstream429RetryOff ?? '429 retry off',
      source: fieldSources.upstream429Retry,
    },
    {
      label: labels.fieldAvailableModels ?? 'Available models',
      value:
        resolvedRule.availableModels && resolvedRule.availableModels.length > 0
          ? resolvedRule.availableModels.join(', ')
          : fieldSources.availableModels === 'tag'
            ? labels.availableModelsNoneAllowed ?? 'No models allowed'
            : labels.availableModelsInherited ?? 'Inherited / unrestricted',
      source: fieldSources.availableModels ?? 'root',
    },
    {
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
            {fieldRows.map((row) => (
              <div
                key={row.label}
                className="grid grid-cols-1 gap-1 border-b border-base-300/60 px-3 py-2.5 text-sm last:border-b-0 sm:grid-cols-[minmax(7rem,1fr)_minmax(8rem,1.2fr)_minmax(5rem,auto)] sm:items-center sm:gap-3"
              >
                <span className="font-medium text-base-content/80">{row.label}</span>
                <span className="text-base-content">{row.value}</span>
                <Badge
                  className="w-fit sm:justify-self-end"
                  variant={row.source === 'account' ? 'default' : row.source === 'tag' ? 'accent' : row.source === 'group' ? 'info' : 'secondary'}
                >
                  {sourceLabel(row.source)}
                </Badge>
              </div>
            ))}
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
