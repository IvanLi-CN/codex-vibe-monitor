import { AppIcon } from "../shared/AppIcon";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Input } from "../../components/ui/input";
import type {
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingTimeoutSettings,
} from "../../lib/api";
import {
  DEFAULT_TIMEOUT_FIELD_SOURCES,
  ROUTING_TIMEOUT_FIELD_ORDER,
  getRoutingTimeoutFieldSource,
  sourceTokenToUiLabel,
  type RoutingTimeoutFieldKey,
  type RoutingTimeoutOverrideDraft,
  type RoutingTimeoutOverrideEnabledState,
} from "../../lib/poolRoutingTimeouts";
import { cn } from "../../lib/utils";

export type RoutingTimeoutEditorLabels = {
  sectionTitle: string;
  sectionHint?: string;
  inheritedValue: string;
  overrideValue: string;
  sourceRoot?: string;
  sourceGroup?: string;
  sourceAccount?: string;
  sourceConversation?: string;
  clearField: string;
  inheritField: string;
  secondsSuffix?: string;
  savingField?: string;
};

export type RoutingTimeoutEditorFieldConfig = {
  key: RoutingTimeoutFieldKey;
  label: string;
};

interface RoutingTimeoutOverridesEditorProps {
  fields: RoutingTimeoutEditorFieldConfig[];
  effective: PoolRoutingTimeoutSettings;
  draft: RoutingTimeoutOverrideDraft;
  enabledFields: RoutingTimeoutOverrideEnabledState;
  sources?: EffectiveRoutingTimeoutFieldSources | null;
  busy?: boolean;
  disabled?: boolean;
  surface?: "framed" | "plain";
  labels: RoutingTimeoutEditorLabels;
  onDraftChange: (key: RoutingTimeoutFieldKey, value: string) => void;
  onFieldEnabledChange: (key: RoutingTimeoutFieldKey, enabled: boolean) => void;
}

function sourceVariant(source: EffectiveRoutingRuleSource) {
  return source === "conversation"
    ? "default"
    : source === "account"
      ? "default"
      : source === "group"
        ? "info"
        : "secondary";
}

export function RoutingTimeoutOverridesEditor({
  fields,
  effective,
  draft,
  enabledFields,
  sources,
  busy = false,
  disabled = false,
  surface = "framed",
  labels,
  onDraftChange,
  onFieldEnabledChange,
}: RoutingTimeoutOverridesEditorProps) {
  const resolvedSources = sources ?? DEFAULT_TIMEOUT_FIELD_SOURCES;
  const resolvedEffective: PoolRoutingTimeoutSettings = {
    responsesFirstByteTimeoutSecs:
      effective?.responsesFirstByteTimeoutSecs ?? 120,
    compactFirstByteTimeoutSecs:
      effective?.compactFirstByteTimeoutSecs ?? 300,
    responsesStreamTimeoutSecs:
      effective?.responsesStreamTimeoutSecs ?? 300,
    compactStreamTimeoutSecs:
      effective?.compactStreamTimeoutSecs ?? 300,
  };

  return (
    <div
      className={cn(
        "space-y-4",
        surface === "framed"
          ? "rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4"
          : "",
      )}
    >
      <div className="space-y-1">
        <p className="font-medium text-base-content">{labels.sectionTitle}</p>
        {labels.sectionHint ? (
          <p className="text-xs leading-5 text-base-content/65">
            {labels.sectionHint}
          </p>
        ) : null}
      </div>
      <div className="mt-4 overflow-hidden rounded-xl border border-base-300/70">
        {fields
          .filter((field) => ROUTING_TIMEOUT_FIELD_ORDER.includes(field.key))
          .map((field) => {
            const rawOverride = draft[field.key] ?? "";
            const source = getRoutingTimeoutFieldSource(resolvedSources, field.key);
            const expanded = enabledFields[field.key] === true;
            const effectiveValue = resolvedEffective[field.key];
            const sourceLabel = sourceTokenToUiLabel(source, {
              root: labels.sourceRoot,
              group: labels.sourceGroup,
              account: labels.sourceAccount,
              conversation: labels.sourceConversation,
            });
            return (
              <div
                key={field.key}
                className="border-b border-base-300/60 last:border-b-0"
              >
                <div className="grid grid-cols-1 gap-1 px-3 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto_2rem] sm:items-center sm:gap-2">
                  <span className="min-w-0 font-medium text-base-content/80">
                    {field.label}
                  </span>
                  <span className="whitespace-nowrap text-base-content">
                    {effectiveValue}
                    {labels.secondsSuffix ?? "s"}
                  </span>
                  <div className="min-w-0 flex flex-wrap items-center gap-2 sm:justify-self-end">
                    <span className="text-xs text-base-content/65">
                      {expanded ? labels.overrideValue : labels.inheritedValue}
                    </span>
                    <Badge
                      className="w-fit"
                      variant={sourceVariant(source)}
                    >
                      {sourceLabel}
                    </Badge>
                  </div>
                  <Button
                    type="button"
                    size="icon"
                    variant={expanded ? "default" : "ghost"}
                    className={cn(
                      "h-8 w-8 justify-self-start rounded-full sm:justify-self-end",
                      expanded ? "text-primary-content" : "text-base-content/65",
                    )}
                    disabled={busy || disabled}
                    aria-pressed={expanded}
                    aria-label={`${expanded ? labels.clearField : labels.inheritField}: ${field.label}`}
                    onClick={() => onFieldEnabledChange(field.key, !expanded)}
                  >
                    <AppIcon
                      name={
                        busy
                          ? "loading"
                          : expanded
                            ? "check-decagram-outline"
                            : "pencil-outline"
                      }
                      className={cn("h-4 w-4", busy ? "animate-spin" : "")}
                      aria-hidden
                    />
                  </Button>
                </div>
                {expanded ? (
                  <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                    <div className="grid grid-cols-1 gap-y-2 sm:grid-cols-[minmax(0,1fr)_auto_auto_2rem] sm:items-center sm:gap-x-2">
                      <p className="min-w-0 text-sm font-semibold text-base-content">
                        {field.label}
                      </p>
                      <div className="min-w-0 sm:col-span-3">
                        <Input
                          name={field.key}
                          type="number"
                          min="1"
                          step="1"
                          value={rawOverride}
                          onChange={(event) =>
                            onDraftChange(field.key, event.target.value)
                          }
                          disabled={busy || disabled}
                          className="h-11 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                        />
                      </div>
                      {busy ? (
                        <p className="text-xs text-base-content/60 sm:col-start-2 sm:col-span-3">
                          {labels.savingField ?? "Saving..."}
                        </p>
                      ) : null}
                    </div>
                  </div>
                ) : null}
              </div>
            );
          })}
      </div>
    </div>
  );
}
