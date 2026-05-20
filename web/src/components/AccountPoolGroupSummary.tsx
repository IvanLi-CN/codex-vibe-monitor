import { AppIcon } from "./AppIcon";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { cn } from "../lib/utils";
import { upstreamPlanBadgeRecipe } from "../lib/upstreamAccountBadges";
import {
  resolveActiveRoutingPolicyBadges,
  type ActiveRoutingPolicyBadgeLabels,
} from "../lib/tagRoutingRule";
import type { AccountPoolGroupSummaryData } from "../lib/accountPoolGroups";
import type { TagRoutingRule } from "../lib/api";

export type AccountPoolGroupSummaryLabels = ActiveRoutingPolicyBadgeLabels & {
  count: (count: number) => string;
  concurrency: (value: number) => string;
  exclusiveNode: string;
  noteLabel: string;
  noteEmpty: string;
  proxiesLabel: string;
  proxiesEmpty: string;
  settingsLabel: string;
  upstream429Enabled: (count: number) => string;
  upstream429Disabled: string;
};

function groupPlanBadgeRecipe(planKey: string) {
  if (planKey === "api") {
    return {
      variant: "info" as const,
      className: undefined,
      dataPlan: undefined,
    };
  }
  return upstreamPlanBadgeRecipe(planKey);
}

function resolveGroupRoutingRule(group: AccountPoolGroupSummaryData): TagRoutingRule {
  const routingRule = group.routingRule;
  return {
    guardEnabled: routingRule?.guardEnabled ?? false,
    lookbackHours: routingRule?.lookbackHours ?? null,
    maxConversations: routingRule?.maxConversations ?? null,
    allowCutOut: routingRule?.allowCutOut ?? true,
    allowCutIn: routingRule?.allowCutIn ?? true,
    priorityTier: routingRule?.priorityTier ?? "normal",
    fastModeRewriteMode: routingRule?.fastModeRewriteMode ?? "keep_original",
    concurrencyLimit: routingRule?.concurrencyLimit ?? group.concurrencyLimit ?? 0,
    upstream429RetryEnabled:
      routingRule?.upstream429RetryEnabled ?? group.upstream429RetryEnabled ?? false,
    upstream429MaxRetries:
      routingRule?.upstream429MaxRetries ?? group.upstream429MaxRetries ?? 0,
  };
}

export function AccountPoolGroupSummary({
  group,
  labels,
  compact = false,
  showNote = false,
  showRetryState = false,
  canEditGroupSettings = false,
  onEditGroupSettings,
}: {
  group: AccountPoolGroupSummaryData;
  labels: AccountPoolGroupSummaryLabels;
  compact?: boolean;
  showNote?: boolean;
  showRetryState?: boolean;
  canEditGroupSettings?: boolean;
  onEditGroupSettings?: (group: AccountPoolGroupSummaryData) => void;
}) {
  const showSettingsAction =
    canEditGroupSettings &&
    Boolean(group.groupName) &&
    typeof onEditGroupSettings === "function";
  const activePolicyBadges = resolveActiveRoutingPolicyBadges(
    resolveGroupRoutingRule(group),
    {
      ...labels,
      policyConcurrency:
        labels.policyConcurrency ?? ((value) => labels.concurrency(value)),
      policyRetry:
        labels.policyRetry ?? ((count) => labels.upstream429Enabled(count)),
    },
  );

  return (
    <div
      className={cn(
        "flex flex-col gap-2.5",
        compact ? "xl:pr-3.5" : "h-full",
        !compact && "xl:border-r xl:border-base-300/65 xl:pr-4",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <h3
              className="min-w-0 text-[16px] font-semibold leading-5 text-base-content"
              title={group.displayName}
            >
              <span className="block truncate">{group.displayName}</span>
            </h3>
            <span className="shrink-0 text-[11px] font-medium leading-4 text-base-content/46">
              {labels.count(group.items.length)}
            </span>
          </div>
        </div>
        {showSettingsAction ? (
          <Button
            type="button"
            size="icon"
            variant={group.hasCustomSettings ? "secondary" : "outline"}
            className="h-9 w-9 shrink-0 rounded-full"
            aria-label={labels.settingsLabel}
            title={labels.settingsLabel}
            onClick={() => onEditGroupSettings?.(group)}
          >
            <AppIcon
              name="file-document-edit-outline"
              className="h-4 w-4"
              aria-hidden
            />
          </Button>
        ) : null}
      </div>

      <div className="flex flex-wrap items-center gap-1.5">
        {group.planCounts.map((plan) => {
          const recipe = groupPlanBadgeRecipe(plan.key);
          return (
            <Badge
              key={plan.key}
              variant={recipe?.variant ?? "secondary"}
              className={cn(
                "shrink-0 whitespace-nowrap px-2 py-px text-[11px] font-medium leading-4",
                recipe?.className,
              )}
              data-plan={recipe?.dataPlan}
            >
              {plan.label} {plan.count}
            </Badge>
          );
        })}
        {activePolicyBadges.map((badge) => (
          <Badge
            key={`policy:${badge.key}`}
            variant={badge.variant}
            className="px-2 py-px text-[11px] font-medium leading-4"
            title={badge.title ?? badge.label}
          >
            {badge.label}
          </Badge>
        ))}
        {group.nodeShuntEnabled ? (
          <Badge
            variant="info"
            className="px-2 py-px text-[11px] font-medium leading-4"
          >
            {labels.exclusiveNode}
          </Badge>
        ) : null}
        {showRetryState && activePolicyBadges.length === 0 ? (
          <Badge variant="secondary" className="px-2 py-px text-[11px] font-medium leading-4">
            {labels.upstream429Disabled}
          </Badge>
        ) : null}
      </div>

      <div className="flex flex-wrap items-center gap-1.5 text-[12px] leading-5 text-base-content/54">
        <span className="shrink-0 font-medium uppercase tracking-[0.12em] text-base-content/42">
          {labels.proxiesLabel}
        </span>
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          {Array.isArray(group.boundProxyLabels) && group.boundProxyLabels.length > 0 ? (
            group.boundProxyLabels.map((label) => (
              <Badge
                key={label}
                variant="secondary"
                className="max-w-full px-2 py-px text-[11px] font-medium leading-4"
                title={label}
              >
                <span className="truncate">{label}</span>
              </Badge>
            ))
          ) : (
            <span className="text-[12px] leading-5 text-base-content/58">
              {labels.proxiesEmpty}
            </span>
          )}
        </div>
      </div>

      {showNote ? (
        <div className="space-y-1">
          <p className="text-[11px] font-medium uppercase tracking-[0.12em] text-base-content/42">
            {labels.noteLabel}
          </p>
          <p className="text-sm leading-6 text-base-content/72">
            {group.note?.trim() || labels.noteEmpty}
          </p>
        </div>
      ) : null}
    </div>
  );
}
