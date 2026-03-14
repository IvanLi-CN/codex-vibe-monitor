import { Badge } from './ui/badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import type { EffectiveRoutingRule } from '../lib/api'

interface EffectiveRoutingRuleCardProps {
  rule?: EffectiveRoutingRule | null
  labels: {
    title: string
    description: string
    noTags: string
    guardEnabled: string
    guardDisabled: string
    allowCutOut: string
    denyCutOut: string
    allowCutIn: string
    denyCutIn: string
    sourceTags: string
    guardRule: (hours: number, count: number) => string
    allGuardsApply: string
  }
}

export function EffectiveRoutingRuleCard({ rule, labels }: EffectiveRoutingRuleCardProps) {
  const resolvedRule: EffectiveRoutingRule = rule ?? {
    guardEnabled: false,
    lookbackHours: null,
    maxConversations: null,
    allowCutOut: true,
    allowCutIn: true,
    sourceTagIds: [],
    sourceTagNames: [],
    guardRules: [],
  }

  return (
    <Card className="border-base-300/80 bg-base-100/72">
      <CardHeader>
        <CardTitle>{labels.title}</CardTitle>
        <CardDescription>{labels.description}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap gap-2">
          <Badge variant={resolvedRule.guardEnabled ? 'warning' : 'secondary'}>
            {resolvedRule.guardEnabled ? labels.guardEnabled : labels.guardDisabled}
          </Badge>
          <Badge variant={resolvedRule.allowCutOut ? 'success' : 'error'}>
            {resolvedRule.allowCutOut ? labels.allowCutOut : labels.denyCutOut}
          </Badge>
          <Badge variant={resolvedRule.allowCutIn ? 'success' : 'error'}>
            {resolvedRule.allowCutIn ? labels.allowCutIn : labels.denyCutIn}
          </Badge>
        </div>

        <div className="rounded-[1.2rem] border border-base-300/70 bg-base-100/70 p-4">
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

        <div className="rounded-[1.2rem] border border-base-300/70 bg-base-100/70 p-4">
          <div className="flex items-center justify-between gap-3">
            <p className="metric-label">{labels.allGuardsApply}</p>
          </div>
          {resolvedRule.guardRules.length === 0 ? (
            <p className="mt-3 text-sm text-base-content/60">{labels.guardDisabled}</p>
          ) : (
            <div className="mt-3 flex flex-wrap gap-2">
              {resolvedRule.guardRules.map((guard) => (
                <Badge key={`${guard.tagId}-${guard.lookbackHours}-${guard.maxConversations}`} variant="warning">
                  {guard.tagName}: {labels.guardRule(guard.lookbackHours, guard.maxConversations)}
                </Badge>
              ))}
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
