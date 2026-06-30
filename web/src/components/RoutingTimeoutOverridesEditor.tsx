import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import type {
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingTimeoutSettings,
} from "../lib/api";
import {
  DEFAULT_TIMEOUT_FIELD_SOURCES,
  ROUTING_TIMEOUT_FIELD_ORDER,
  getRoutingTimeoutFieldSource,
  sourceTokenToUiLabel,
  type RoutingTimeoutFieldKey,
  type RoutingTimeoutOverrideDraft,
} from "../lib/poolRoutingTimeouts";

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
};

export type RoutingTimeoutEditorFieldConfig = {
  key: RoutingTimeoutFieldKey;
  label: string;
};

interface RoutingTimeoutOverridesEditorProps {
  fields: RoutingTimeoutEditorFieldConfig[];
  effective: PoolRoutingTimeoutSettings;
  draft: RoutingTimeoutOverrideDraft;
  sources?: EffectiveRoutingTimeoutFieldSources | null;
  busy?: boolean;
  disabled?: boolean;
  labels: RoutingTimeoutEditorLabels;
  onDraftChange: (key: RoutingTimeoutFieldKey, value: string) => void;
  onClearField?: (key: RoutingTimeoutFieldKey) => void;
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
  sources,
  busy = false,
  disabled = false,
  labels,
  onDraftChange,
  onClearField,
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
    <div className="rounded-[1.25rem] border border-base-300/80 bg-base-100/80 p-4">
      <div className="space-y-1">
        <p className="font-medium text-base-content">{labels.sectionTitle}</p>
        {labels.sectionHint ? (
          <p className="text-xs leading-5 text-base-content/65">
            {labels.sectionHint}
          </p>
        ) : null}
      </div>
      <div className="mt-4 grid gap-3">
        {fields
          .filter((field) => ROUTING_TIMEOUT_FIELD_ORDER.includes(field.key))
          .map((field) => {
            const rawOverride = draft[field.key] ?? "";
            const source = getRoutingTimeoutFieldSource(resolvedSources, field.key);
            const hasOverride = rawOverride.trim() !== "";
            const effectiveValue = resolvedEffective[field.key];
            return (
              <div
                key={field.key}
                className="rounded-xl border border-base-300/70 bg-base-200/35 p-3"
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="space-y-1">
                    <p className="text-sm font-medium text-base-content">
                      {field.label}
                    </p>
                    <div className="flex flex-wrap items-center gap-2 text-xs text-base-content/65">
                      <span>
                        {hasOverride
                          ? labels.overrideValue
                          : labels.inheritedValue}
                        : {effectiveValue}
                        {labels.secondsSuffix ?? "s"}
                      </span>
                      <Badge variant={sourceVariant(source)}>
                        {sourceTokenToUiLabel(source, {
                          root: labels.sourceRoot,
                          group: labels.sourceGroup,
                          account: labels.sourceAccount,
                          conversation: labels.sourceConversation,
                        })}
                      </Badge>
                    </div>
                  </div>
                  <div className="flex gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={busy || disabled || !hasOverride}
                      onClick={() => {
                        if (onClearField) {
                          onClearField(field.key)
                          return
                        }
                        onDraftChange(field.key, '')
                      }}
                    >
                      {hasOverride ? labels.clearField : labels.inheritField}
                    </Button>
                    {hasOverride && onClearField ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        disabled={busy || disabled}
                        onClick={() => onClearField(field.key)}
                      >
                        {labels.clearField}
                      </Button>
                    ) : null}
                  </div>
                </div>
                <div className="mt-3">
                  <Input
                    name={field.key}
                    type="number"
                    min="1"
                    step="1"
                    value={rawOverride}
                    onChange={(event) => onDraftChange(field.key, event.target.value)}
                    disabled={busy || disabled}
                    className="h-11 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                  />
                </div>
              </div>
            );
          })}
      </div>
    </div>
  );
}
