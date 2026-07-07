import { useEffect, useMemo, useRef, useState } from "react";
import { AppIcon } from "../shared/AppIcon";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "../../components/ui/command";
import { Input } from "../../components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "../../components/ui/popover";
import { Switch } from "../../components/ui/switch";
import { PolicyInlineOptionGroup } from "./PolicyInlineOptionGroup";
import { StatusChangeToggleButton } from "./StatusChangeToggleButton";
import { statusChangeReasonIconName } from "./statusChangeReasonIcons";
import type {
  EffectiveRoutingRule,
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  ImageToolRewriteMode,
  PoolRoutingTimeoutSettings,
  TagFastModeRewriteMode,
  TagPriorityTier,
  UpdateGroupAccountRoutingRulePayload,
} from "../../lib/api";
import {
  CONCURRENCY_LIMIT_MAX,
  CONCURRENCY_LIMIT_MIN,
  CONCURRENCY_LIMIT_UNLIMITED_SLIDER_VALUE,
  apiConcurrencyLimitToSliderValue,
  formatConcurrencyLimitValue,
  sliderConcurrencyLimitToApiValue,
} from "../../lib/concurrencyLimit";
import {
  fastModeRewriteBadgeLabel,
  priorityTierBadgeLabel,
} from "../../lib/tagRoutingRule";
import {
  ROUTING_TIMEOUT_FIELD_ORDER,
  type RoutingTimeoutFieldKey,
} from "../../lib/poolRoutingTimeouts";
import {
  STATUS_CHANGE_REASON_CODES,
  countEnabledStatusChangeReasons,
  resolveStatusChangeReasonFieldSources,
  resolveStatusChangeReasons,
  statusChangeReasonFieldKey,
  statusChangeReasonFromFieldKey,
  type StatusChangeReasonCode,
  type StatusChangeReasonFieldKey,
  type StatusChangeReasonFieldSources,
} from "../../lib/upstreamAccountStatusChangeReasons";
import { cn } from "../../lib/utils";

type StatusChangeEditablePolicyField = StatusChangeReasonFieldKey;

type EditablePolicyField =
  | "allowCutOut"
  | "allowCutIn"
  | "priorityTier"
  | "fastModeRewriteMode"
  | "imageToolRewriteMode"
  | "concurrencyLimit"
  | "upstream429Retry"
  | "availableModels"
  | "timeoutResponsesFirstByte"
  | "timeoutCompactFirstByte"
  | "timeoutResponsesStream"
  | "timeoutCompactStream"
  | "statusChangeReasons"
  | "proxyBindings"
  | StatusChangeEditablePolicyField;

type FieldSourceMap = NonNullable<EffectiveRoutingRule["fieldSources"]>;

const editableFieldSourceKeys: Array<
  [EditablePolicyField, keyof FieldSourceMap]
> = [
  ["allowCutOut", "allowCutOut"],
  ["allowCutIn", "allowCutIn"],
  ["priorityTier", "priorityTier"],
  ["fastModeRewriteMode", "fastModeRewriteMode"],
  ["imageToolRewriteMode", "imageToolRewriteMode"],
  ["concurrencyLimit", "concurrencyLimit"],
  ["upstream429Retry", "upstream429Retry"],
  ["availableModels", "availableModels"],
];

interface EditablePolicyConfig {
  busyField?: EditablePolicyField | null;
  errorByField?: Partial<Record<EditablePolicyField, string | null>>;
  availableModelOptions?: string[];
  onChange: (
    field: EditablePolicyField,
    payload: UpdateGroupAccountRoutingRulePayload,
  ) => Promise<void> | void;
}

interface EffectiveProxyBindingItem {
  key: string;
  label: string;
  status?: string;
  accountOverride?: boolean;
  tone?: "direct" | "available" | "unavailable" | "missing";
}

interface EffectiveProxyBindingConfig {
  source: "account" | "group";
  items: EffectiveProxyBindingItem[];
  busy?: boolean;
  disabled?: boolean;
  onEdit?: () => void;
  onClear?: () => void;
  onRemove?: (key: string) => void;
  labels: {
    field: string;
    add: string;
    clear: string;
    empty: string;
    hint: string;
    remove: string;
  };
}

interface EffectiveRoutingRuleCardProps {
  rule?: EffectiveRoutingRule | null;
  identityKey?: string | number | null;
  editablePolicy?: EditablePolicyConfig;
  proxyBindings?: EffectiveProxyBindingConfig;
  labels: {
    title: string;
    description: string;
    noTags: string;
    allowCutOut: string;
    denyCutOut: string;
    allowCutIn: string;
    denyCutIn: string;
    sourceTags: string;
    priorityPrimary: string;
    priorityNormal: string;
    priorityFallback: string;
    priorityNoNew?: string;
    fastModeKeepOriginal: string;
    fastModeFillMissing: string;
    fastModeForceAdd: string;
    fastModeForceRemove: string;
    imageToolKeepOriginal: string;
    imageToolFillMissing: string;
    imageToolForceAdd: string;
    imageToolForceRemove: string;
    availableModelsInherited?: string;
    availableModelsNoneAllowed?: string;
    availableModelsEmpty?: string;
    availableModelsField?: string;
    systemDeniedModelsField?: string;
    systemDeniedModelsEmpty?: string;
    concurrencyLimit?: (count: number) => string;
    concurrencyUnlimited?: string;
    sourceBreakdownTitle?: string;
    fieldAllowCutOut?: string;
    fieldAllowCutIn?: string;
    fieldPriority?: string;
    fieldFastMode?: string;
    fieldImageToolRewriteMode?: string;
    fieldConcurrency?: string;
    fieldUpstream429?: string;
    fieldAvailableModels?: string;
    fieldSystemDeniedModels?: string;
    fieldProxyBindings?: string;
    statusChangeReasonSectionTitle?: string;
    statusChangeReasonSectionHint?: string;
    statusChangeReasonLabel?: (reason: StatusChangeReasonCode) => string;
    statusChangeReasonSummary?: (enabled: number, total: number) => string;
    statusChangeReasonEnabledValue?: string;
    statusChangeReasonDisabledValue?: string;
    statusChangeReasonToggleEnabled?: string;
    statusChangeReasonToggleDisabled?: string;
    statusChangeReasonResetAction?: string;
    timeoutSectionTitle?: string;
    timeoutInheritedValue?: string;
    timeoutOverrideValue?: string;
    timeoutResponsesFirstByte?: string;
    timeoutCompactFirstByte?: string;
    timeoutResponsesStream?: string;
    timeoutCompactStream?: string;
    sourceRoot?: string;
    sourceGroup?: string;
    sourceTag?: string;
    sourceAccount?: string;
    sourceConversation?: string;
    sourceSystem?: string;
    overrideEdit?: string;
    overrideActive?: string;
    overrideClear?: string;
    overrideSaving?: string;
    inheritValue?: string;
    allowLabel?: string;
    denyLabel?: string;
    newConversationLabel?: string;
    cutOutLabel?: string;
    cutInLabel?: string;
    upstream429RetryCountValue?: (count: number) => string;
    availableModelsAddCustom?: string;
    availableModelsCustomLabel?: (value: string) => string;
    availableModelsRemove?: string;
    availableModelsPlaceholder?: string;
    currentValue?: string;
  };
}

const defaultFieldSources: FieldSourceMap = {
  allowCutOut: "root",
  allowCutIn: "root",
  priorityTier: "root",
  fastModeRewriteMode: "root",
  imageToolRewriteMode: "root",
  concurrencyLimit: "root",
  upstream429Retry: "root",
  availableModels: "root",
  systemDeniedModels: "root",
};

function defaultRule(rule?: EffectiveRoutingRule | null): EffectiveRoutingRule {
  return (
    rule ?? {
      allowCutOut: true,
      allowCutIn: true,
      priorityTier: "normal",
      fastModeRewriteMode: "keep_original",
      imageToolRewriteMode: "keep_original",
      sourceTagIds: [],
      sourceTagNames: [],
      concurrencyLimit: 0,
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
      availableModels: [],
      systemDeniedModels: [],
      statusChangeReasons: resolveStatusChangeReasons(null),
      statusChangeReasonFieldSources:
        resolveStatusChangeReasonFieldSources(null),
      fieldSources: defaultFieldSources,
      timeouts: {
        responsesFirstByteTimeoutSecs: 120,
        compactFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
      timeoutFieldSources: {
        responsesFirstByteTimeoutSecs: "root",
        compactFirstByteTimeoutSecs: "root",
        responsesStreamTimeoutSecs: "root",
        compactStreamTimeoutSecs: "root",
      },
    }
  );
}

function sourceLabel(
  source: string,
  labels: EffectiveRoutingRuleCardProps["labels"],
): string {
  switch (source) {
    case "root":
      return labels.sourceRoot ?? "Root default";
    case "group":
      return labels.sourceGroup ?? "Group";
    case "tag":
      return labels.sourceTag ?? "Tag";
    case "account":
      return labels.sourceAccount ?? "Account";
    case "conversation":
      return labels.sourceConversation ?? "Conversation";
    case "system":
      return labels.sourceSystem ?? "System";
    default:
      return source;
  }
}

function sourceVariant(source: string) {
  return source === "account" || source === "conversation"
    ? "default"
    : source === "tag"
      ? "accent"
      : source === "group"
        ? "info"
        : "secondary";
}

type BadgeVariant = React.ComponentProps<typeof Badge>["variant"];

function statusChangeDisabledValue(
  labels: EffectiveRoutingRuleCardProps["labels"],
) {
  return labels.statusChangeReasonDisabledValue ?? "Evidence only";
}

function valueVariant(
  field: EditablePolicyField | null,
  value: string,
  labels: EffectiveRoutingRuleCardProps["labels"],
): BadgeVariant {
  if (field && statusChangeReasonFromFieldKey(field)) {
    return value === statusChangeDisabledValue(labels) ? "success" : "warning";
  }
  if (field === "allowCutOut") {
    return value === labels.denyCutOut ? "warning" : "success";
  }
  if (field === "allowCutIn") {
    return value === labels.denyCutIn ? "warning" : "success";
  }
  if (field === "priorityTier") {
    if (value === (labels.priorityNoNew ?? "No new")) return "warning";
    if (value === labels.priorityPrimary) return "default";
    if (value === labels.priorityFallback) return "warning";
    return "info";
  }
  if (field === "fastModeRewriteMode") {
    if (
      value === labels.fastModeForceAdd ||
      value === labels.fastModeForceRemove
    )
      return "default";
    if (value === labels.fastModeFillMissing) return "info";
    return "secondary";
  }
  if (field === "imageToolRewriteMode") {
    if (
      value === labels.imageToolForceAdd ||
      value === labels.imageToolForceRemove
    )
      return "default";
    if (value === labels.imageToolFillMissing) return "info";
    return "secondary";
  }
  if (field === "concurrencyLimit") {
    return value === (labels.concurrencyUnlimited ?? "Concurrency unlimited")
      ? "success"
      : "warning";
  }
  if (field === "upstream429Retry") {
    return value === "0" ? "secondary" : "info";
  }
  if (field === "availableModels") {
    if (
      value ===
        (labels.availableModelsInherited ?? "Inherited / unrestricted") ||
      value === (labels.availableModelsNoneAllowed ?? "No models allowed")
    ) {
      return value ===
        (labels.availableModelsNoneAllowed ?? "No models allowed")
        ? "warning"
        : "success";
    }
    return "default";
  }
  if (field == null && value === (labels.systemDeniedModelsEmpty ?? "None")) {
    return "success";
  }
  return field == null ? "warning" : "secondary";
}

function ValueBadge({
  field,
  value,
  labels,
}: {
  field: EditablePolicyField | null;
  value: string;
  labels: EffectiveRoutingRuleCardProps["labels"];
}) {
  return (
    <Badge
      className="min-w-0 max-w-full justify-self-start whitespace-normal break-words text-left leading-5"
      variant={valueVariant(field, value, labels)}
    >
      {value}
    </Badge>
  );
}

function ValueBadgeList({
  field,
  values,
  labels,
}: {
  field: EditablePolicyField | null;
  values: string[];
  labels: EffectiveRoutingRuleCardProps["labels"];
}) {
  return (
    <div className="flex min-w-0 flex-wrap gap-2 justify-self-start">
      {values.map((value) => (
        <ValueBadge key={value} field={field} value={value} labels={labels} />
      ))}
    </div>
  );
}

function ProxyBindingChips({
  items,
  labels,
  disabled,
  onRemove,
}: {
  items: EffectiveProxyBindingItem[];
  labels: EffectiveProxyBindingConfig["labels"];
  disabled?: boolean;
  onRemove?: (key: string) => void;
}) {
  if (items.length === 0) {
    return <span className="text-sm text-base-content/60">{labels.empty}</span>;
  }
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      {items.map((item) => (
        <span
          key={item.key}
          className={cn(
            "inline-flex min-w-0 max-w-full items-center gap-2 rounded-full border px-2.5 py-1 text-xs",
            item.tone === "direct"
              ? "border-primary/40 bg-primary/10 text-primary"
              : item.tone === "missing"
                ? "border-error/35 bg-error/15 text-error"
                : item.tone === "available"
                  ? "border-success/35 bg-success/15 text-success"
                  : "border-base-300 bg-base-200/70 text-base-content/85",
          )}
        >
          <span className="max-w-56 truncate font-medium">{item.label}</span>
          {item.status ? (
            <span className="shrink-0 text-current/70">{item.status}</span>
          ) : null}
          {item.accountOverride && onRemove ? (
            <button
              type="button"
              className="shrink-0 rounded-full px-1 text-current/60 hover:bg-base-200 hover:text-current disabled:cursor-not-allowed disabled:opacity-50"
              disabled={disabled}
              onClick={(event) => {
                event.stopPropagation();
                onRemove(item.key);
              }}
              aria-label={labels.remove}
            >
              x
            </button>
          ) : null}
        </span>
      ))}
    </div>
  );
}

function ProxyBindingMultiSelectTrigger({
  items,
  labels,
  disabled,
  onOpen,
  onRemove,
}: {
  items: EffectiveProxyBindingItem[];
  labels: EffectiveProxyBindingConfig["labels"];
  disabled?: boolean;
  onOpen?: () => void;
  onRemove?: (key: string) => void;
}) {
  return (
    <div
      role="combobox"
      aria-expanded={false}
      aria-label={labels.field}
      aria-disabled={disabled ? "true" : undefined}
      tabIndex={disabled ? -1 : 0}
      title={
        items.length > 0
          ? items.map((item) => item.label).join(", ")
          : labels.empty
      }
      className={cn(
        "flex min-h-11 w-full items-center gap-3 rounded-xl border border-base-300 bg-base-100 px-3 py-2.5 text-left shadow-sm transition-colors",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
        "hover:border-primary/35",
        disabled && "cursor-not-allowed opacity-60",
      )}
      onClick={() => {
        if (!disabled) onOpen?.();
      }}
      onKeyDown={(event) => {
        if (disabled) return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen?.();
        }
      }}
    >
      <AppIcon
        name="tag-outline"
        className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55"
        aria-hidden
      />
      <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
        <ProxyBindingChips
          items={items}
          labels={labels}
          disabled={disabled}
          onRemove={onRemove}
        />
      </span>
      <AppIcon
        name="chevron-down"
        className="h-4 w-4 shrink-0 text-base-content/45"
        aria-hidden
      />
    </div>
  );
}

function accountOverrideFields(
  fieldSources: FieldSourceMap,
): EditablePolicyField[] {
  return editableFieldSourceKeys
    .filter(([, sourceKey]) => fieldSources[sourceKey] === "account")
    .map(([field]) => field);
}

function accountStatusChangeOverrideFields(
  reasonSources: StatusChangeReasonFieldSources,
): EditablePolicyField[] {
  return STATUS_CHANGE_REASON_CODES.filter(
    (reason) => reasonSources[reason] === "account",
  ).map((reason) => statusChangeReasonFieldKey(reason));
}

const timeoutFieldToInlineField: Record<
  RoutingTimeoutFieldKey,
  EditablePolicyField
> = {
  responsesFirstByteTimeoutSecs: "timeoutResponsesFirstByte",
  compactFirstByteTimeoutSecs: "timeoutCompactFirstByte",
  responsesStreamTimeoutSecs: "timeoutResponsesStream",
  compactStreamTimeoutSecs: "timeoutCompactStream",
};

function accountTimeoutOverrideFields(
  timeoutSources: EffectiveRoutingTimeoutFieldSources,
): EditablePolicyField[] {
  return ROUTING_TIMEOUT_FIELD_ORDER.filter(
    (key) => timeoutSources[key] === "account",
  ).map((key) => timeoutFieldToInlineField[key]);
}

function normalizeModelIds(values: string[]) {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
}

function formatUpstream429RetryCount(
  count: number,
  labels: EffectiveRoutingRuleCardProps["labels"],
) {
  const normalized = Math.min(5, Math.max(0, Math.trunc(count)));
  return labels.upstream429RetryCountValue?.(normalized) ?? String(normalized);
}

export function EffectiveRoutingRuleCard({
  rule,
  identityKey,
  labels,
  editablePolicy,
  proxyBindings,
}: EffectiveRoutingRuleCardProps) {
  const resolvedRule = defaultRule(rule);
  const isEditable = editablePolicy != null;
  const fieldSources = useMemo(
    () => ({ ...defaultFieldSources, ...(resolvedRule.fieldSources ?? {}) }),
    [resolvedRule.fieldSources],
  );
  const timeoutSources = useMemo<EffectiveRoutingTimeoutFieldSources>(
    () =>
      resolvedRule.timeoutFieldSources ?? {
        responsesFirstByteTimeoutSecs: "root",
        compactFirstByteTimeoutSecs: "root",
        responsesStreamTimeoutSecs: "root",
        compactStreamTimeoutSecs: "root",
      },
    [resolvedRule.timeoutFieldSources],
  );
  const timeoutValues = useMemo<PoolRoutingTimeoutSettings>(
    () =>
      resolvedRule.timeouts ?? {
        responsesFirstByteTimeoutSecs: 120,
        compactFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
    [resolvedRule.timeouts],
  );
  const statusChangeReasons = useMemo(
    () => resolveStatusChangeReasons(resolvedRule.statusChangeReasons),
    [resolvedRule.statusChangeReasons],
  );
  const statusChangeReasonSources = useMemo(
    () =>
      resolveStatusChangeReasonFieldSources(
        resolvedRule.statusChangeReasonFieldSources,
      ),
    [resolvedRule.statusChangeReasonFieldSources],
  );
  const defaultExpandedFields = isEditable
    ? [
        ...accountOverrideFields(fieldSources),
        ...accountStatusChangeOverrideFields(statusChangeReasonSources),
        ...accountTimeoutOverrideFields(timeoutSources),
        ...(proxyBindings?.source === "account"
          ? (["proxyBindings"] as const)
          : []),
      ]
    : [];
  const [expandedFields, setExpandedFields] = useState<EditablePolicyField[]>(
    defaultExpandedFields,
  );
  const [availableModelInput, setAvailableModelInput] = useState("");
  const userTouchedExpansionRef = useRef(false);
  const previousIdentityKeyRef = useRef(identityKey);

  useEffect(() => {
    const identityChanged = previousIdentityKeyRef.current !== identityKey;
    if (identityChanged) {
      previousIdentityKeyRef.current = identityKey;
      userTouchedExpansionRef.current = false;
    }

    if (!isEditable) {
      userTouchedExpansionRef.current = false;
      setExpandedFields([]);
      return;
    }

    const nextDefaultExpandedFields = [
      ...accountOverrideFields(fieldSources),
      ...accountStatusChangeOverrideFields(statusChangeReasonSources),
      ...accountTimeoutOverrideFields(timeoutSources),
      ...(proxyBindings?.source === "account"
        ? (["proxyBindings"] as const)
        : []),
    ];
    setExpandedFields((current) => {
      if (userTouchedExpansionRef.current) return current;
      if (
        current.some((field) =>
          field === "proxyBindings"
            ? proxyBindings?.source === "account"
            : fieldToSource(
                field,
                fieldSources,
                timeoutSources,
                statusChangeReasonSources,
              ) === "account",
        )
      )
        return current;
      return nextDefaultExpandedFields;
    });
  }, [
    isEditable,
    identityKey,
    fieldSources,
    statusChangeReasonSources,
    timeoutSources,
    proxyBindings?.source,
  ]);

  const availableModelOptions = useMemo(
    () =>
      normalizeModelIds([
        ...(editablePolicy?.availableModelOptions ?? []),
        ...(resolvedRule.availableModels ?? []),
      ]),
    [editablePolicy?.availableModelOptions, resolvedRule.availableModels],
  );

  const isBusy = (field: EditablePolicyField) =>
    editablePolicy?.busyField === field;
  const changeField = (
    field: EditablePolicyField,
    payload: UpdateGroupAccountRoutingRulePayload,
  ) => {
    void editablePolicy?.onChange(field, payload);
  };
  const clearField = (
    field: EditablePolicyField,
    payload: UpdateGroupAccountRoutingRulePayload,
  ) => {
    userTouchedExpansionRef.current = true;
    changeField(field, payload);
    setExpandedFields((current) => current.filter((value) => value !== field));
  };
  const toggleExpanded = (
    field: EditablePolicyField,
    clearPayload: UpdateGroupAccountRoutingRulePayload,
  ) => {
    userTouchedExpansionRef.current = true;
    const active =
      fieldToSource(
        field,
        fieldSources,
        timeoutSources,
        statusChangeReasonSources,
      ) === "account";
    if (active) {
      clearField(field, clearPayload);
      return;
    }
    setExpandedFields((current) =>
      current.includes(field)
        ? current.filter((value) => value !== field)
        : [...current, field],
    );
  };

  const availableModelsValue = normalizeModelIds(
    resolvedRule.availableModels ?? [],
  );
  const updateAvailableModels = (nextModels: string[]) => {
    changeField("availableModels", {
      availableModels: normalizeModelIds(nextModels),
    });
  };
  const appendAvailableModel = (model: string) => {
    const trimmed = model.trim();
    if (!trimmed || availableModelsValue.includes(trimmed)) return;
    updateAvailableModels([...availableModelsValue, trimmed]);
    setAvailableModelInput("");
  };
  const upstream429RetryCount =
    resolvedRule.upstream429RetryEnabled === true
      ? Math.min(
          5,
          Math.max(0, Math.trunc(resolvedRule.upstream429MaxRetries ?? 0)),
        )
      : 0;
  const inlineTimeoutBusy = ROUTING_TIMEOUT_FIELD_ORDER.some(
    (key) => editablePolicy?.busyField === timeoutFieldToInlineField[key],
  );
  const timeoutRows = ROUTING_TIMEOUT_FIELD_ORDER.map((key) => {
    const field = timeoutFieldToInlineField[key];
    const source = timeoutSources[key];
    const label =
      key === "responsesFirstByteTimeoutSecs"
        ? (labels.timeoutResponsesFirstByte ??
          "Standard response first byte timeout")
        : key === "compactFirstByteTimeoutSecs"
          ? (labels.timeoutCompactFirstByte ??
            "Compact response first byte timeout")
          : key === "responsesStreamTimeoutSecs"
            ? (labels.timeoutResponsesStream ??
              "Standard stream completion timeout")
            : (labels.timeoutCompactStream ??
              "Compact stream completion timeout");
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
    };
  });

  const fieldRows = [
    {
      field: "priorityTier" as const,
      label: labels.fieldPriority ?? "Priority",
      value: priorityTierBadgeLabel(resolvedRule.priorityTier, labels),
      source: fieldSources.priorityTier,
      clearPayload: { priorityTier: null },
      editor: (
        <PolicyInlineOptionGroup<TagPriorityTier>
          ariaLabel={labels.fieldPriority ?? "Priority"}
          value={resolvedRule.priorityTier ?? "normal"}
          disabled={isBusy("priorityTier")}
          options={[
            { value: "primary", label: labels.priorityPrimary },
            { value: "normal", label: labels.priorityNormal },
            { value: "fallback", label: labels.priorityFallback },
            { value: "no_new", label: labels.priorityNoNew ?? "No new" },
          ]}
          onChange={(value) =>
            changeField("priorityTier", { priorityTier: value })
          }
        />
      ),
    },
    {
      field: "allowCutOut" as const,
      label: labels.cutOutLabel ?? labels.fieldAllowCutOut ?? "Cut out",
      value: resolvedRule.allowCutOut ? labels.allowCutOut : labels.denyCutOut,
      source: fieldSources.allowCutOut,
      clearPayload: { allowCutOut: null },
      editor: (
        <Switch
          checked={resolvedRule.allowCutOut}
          disabled={isBusy("allowCutOut")}
          onCheckedChange={(checked) =>
            changeField("allowCutOut", { allowCutOut: checked })
          }
          aria-label={labels.cutOutLabel ?? "Cut out"}
        />
      ),
    },
    {
      field: "allowCutIn" as const,
      label: labels.cutInLabel ?? labels.fieldAllowCutIn ?? "Cut in",
      value: resolvedRule.allowCutIn ? labels.allowCutIn : labels.denyCutIn,
      source: fieldSources.allowCutIn,
      clearPayload: { allowCutIn: null },
      editor: (
        <Switch
          checked={resolvedRule.allowCutIn}
          disabled={isBusy("allowCutIn")}
          onCheckedChange={(checked) =>
            changeField("allowCutIn", { allowCutIn: checked })
          }
          aria-label={labels.cutInLabel ?? "Cut in"}
        />
      ),
    },
    {
      field: "fastModeRewriteMode" as const,
      label: labels.fieldFastMode ?? "FAST mode",
      value: fastModeRewriteBadgeLabel(
        resolvedRule.fastModeRewriteMode,
        labels,
      ),
      source: fieldSources.fastModeRewriteMode,
      clearPayload: { fastModeRewriteMode: null },
      editor: (
        <PolicyInlineOptionGroup<TagFastModeRewriteMode>
          ariaLabel={labels.fieldFastMode ?? "FAST mode"}
          value={resolvedRule.fastModeRewriteMode ?? "keep_original"}
          disabled={isBusy("fastModeRewriteMode")}
          options={[
            { value: "keep_original", label: labels.fastModeKeepOriginal },
            { value: "fill_missing", label: labels.fastModeFillMissing },
            { value: "force_add", label: labels.fastModeForceAdd },
            { value: "force_remove", label: labels.fastModeForceRemove },
          ]}
          onChange={(value) =>
            changeField("fastModeRewriteMode", { fastModeRewriteMode: value })
          }
        />
      ),
    },
    {
      field: "imageToolRewriteMode" as const,
      label: labels.fieldImageToolRewriteMode ?? "Image tools",
      value:
        resolvedRule.imageToolRewriteMode === "fill_missing"
          ? labels.imageToolFillMissing
          : resolvedRule.imageToolRewriteMode === "force_add"
            ? labels.imageToolForceAdd
            : resolvedRule.imageToolRewriteMode === "force_remove"
              ? labels.imageToolForceRemove
              : labels.imageToolKeepOriginal,
      source: fieldSources.imageToolRewriteMode ?? "root",
      clearPayload: { imageToolRewriteMode: null },
      editor: (
        <PolicyInlineOptionGroup<ImageToolRewriteMode>
          ariaLabel={labels.fieldImageToolRewriteMode ?? "Image tools"}
          value={resolvedRule.imageToolRewriteMode ?? "keep_original"}
          disabled={isBusy("imageToolRewriteMode")}
          options={[
            { value: "keep_original", label: labels.imageToolKeepOriginal },
            { value: "fill_missing", label: labels.imageToolFillMissing },
            { value: "force_add", label: labels.imageToolForceAdd },
            { value: "force_remove", label: labels.imageToolForceRemove },
          ]}
          onChange={(value) =>
            changeField("imageToolRewriteMode", { imageToolRewriteMode: value })
          }
        />
      ),
    },
    {
      field: "concurrencyLimit" as const,
      label: labels.fieldConcurrency ?? "Concurrency",
      value: resolvedRule.concurrencyLimit
        ? (labels.concurrencyLimit?.(resolvedRule.concurrencyLimit) ??
          `Concurrency ${resolvedRule.concurrencyLimit}`)
        : (labels.concurrencyUnlimited ?? "Concurrency unlimited"),
      source: fieldSources.concurrencyLimit,
      clearPayload: { concurrencyLimit: null },
      editor: (
        <ConcurrencyInlineEditor
          value={resolvedRule.concurrencyLimit ?? 0}
          disabled={isBusy("concurrencyLimit")}
          currentLabel={labels.currentValue ?? "Current"}
          unlimitedLabel={labels.concurrencyUnlimited ?? "Unlimited"}
          onChange={(value) =>
            changeField("concurrencyLimit", { concurrencyLimit: value })
          }
        />
      ),
    },
    {
      field: "upstream429Retry" as const,
      label: labels.fieldUpstream429 ?? "Upstream 429 retry",
      value: formatUpstream429RetryCount(upstream429RetryCount, labels),
      source: fieldSources.upstream429Retry,
      clearPayload: {
        upstream429RetryEnabled: null,
        upstream429MaxRetries: null,
      },
      editor: (
        <RetryInlineEditor
          retries={upstream429RetryCount}
          disabled={isBusy("upstream429Retry")}
          labels={labels}
          onChange={(count) =>
            changeField("upstream429Retry", {
              upstream429RetryEnabled: count > 0,
              upstream429MaxRetries: count,
            })
          }
        />
      ),
    },
    {
      field: "availableModels" as const,
      label: labels.fieldAvailableModels ?? "Available models",
      value:
        availableModelsValue.length > 0
          ? availableModelsValue.join(", ")
          : fieldSources.availableModels === "account" ||
              fieldSources.availableModels === "group" ||
              fieldSources.availableModels === "tag"
            ? (labels.availableModelsNoneAllowed ?? "No models allowed")
            : (labels.availableModelsInherited ?? "Inherited / unrestricted"),
      source: fieldSources.availableModels ?? "root",
      valueBadges:
        availableModelsValue.length > 0 ? availableModelsValue : null,
      clearPayload: { availableModels: null },
      editor: (
        <AvailableModelsEditor
          value={availableModelsValue}
          options={availableModelOptions}
          inputValue={availableModelInput}
          emptyValueLabel={
            fieldSources.availableModels === "account" ||
            fieldSources.availableModels === "group" ||
            fieldSources.availableModels === "tag"
              ? (labels.availableModelsNoneAllowed ?? "No models allowed")
              : (labels.availableModelsInherited ?? "Inherited / unrestricted")
          }
          disabled={isBusy("availableModels")}
          labels={labels}
          onInputChange={setAvailableModelInput}
          onAdd={appendAvailableModel}
          onChange={updateAvailableModels}
        />
      ),
    },
    {
      field: null,
      label: labels.fieldSystemDeniedModels ?? "System denied models",
      value:
        resolvedRule.systemDeniedModels &&
        resolvedRule.systemDeniedModels.length > 0
          ? resolvedRule.systemDeniedModels.join(", ")
          : (labels.systemDeniedModelsEmpty ?? "None"),
      source: fieldSources.systemDeniedModels ?? "root",
      valueBadges:
        resolvedRule.systemDeniedModels &&
        resolvedRule.systemDeniedModels.length > 0
          ? resolvedRule.systemDeniedModels
          : null,
    },
  ];
  const statusChangeReasonRows = STATUS_CHANGE_REASON_CODES.map((reason) => {
    const field = statusChangeReasonFieldKey(reason);
    return {
      reason,
      field,
      label: labels.statusChangeReasonLabel?.(reason) ?? reason,
      enabled: statusChangeReasons[reason],
      source: statusChangeReasonSources[reason],
      clearPayload: {
        statusChangeReasons: {
          [reason]: null,
        },
      } satisfies UpdateGroupAccountRoutingRulePayload,
    };
  });
  const totalEnabledStatusChangeReasons =
    countEnabledStatusChangeReasons(statusChangeReasons);
  const statusChangeReasonAccountOverrideReasons =
    STATUS_CHANGE_REASON_CODES.filter(
      (reason) => statusChangeReasonSources[reason] === "account",
    );
  const statusChangeReasonHasAccountOverrides =
    statusChangeReasonAccountOverrideReasons.length > 0;
  const statusChangeReasonResetBusy = isBusy("statusChangeReasons");
  const statusChangeReasonResetPayload =
    statusChangeReasonHasAccountOverrides
      ? ({
          statusChangeReasons: Object.fromEntries(
            statusChangeReasonAccountOverrideReasons.map((reason) => [
              reason,
              null,
            ]),
          ),
        } satisfies UpdateGroupAccountRoutingRulePayload)
      : null;
  const statusChangeReasonSectionError =
    editablePolicy?.errorByField?.statusChangeReasons ?? null;
  const proxyBindingsSource = proxyBindings?.source ?? "group";
  const proxyBindingsActiveOverride = proxyBindingsSource === "account";
  const proxyBindingsExpanded = expandedFields.includes("proxyBindings");
  const toggleProxyBindingsRow = () => {
    if (!proxyBindings || proxyBindings.busy) return;
    userTouchedExpansionRef.current = true;
    if (proxyBindingsActiveOverride) {
      proxyBindings.onClear?.();
      setExpandedFields((current) =>
        current.filter((value) => value !== "proxyBindings"),
      );
      return;
    }
    proxyBindings.onEdit?.();
  };

  return (
    <Card className="border-base-300/80 bg-base-100/72">
      <CardHeader>
        <CardTitle>{labels.title}</CardTitle>
        <CardDescription>{labels.description}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-3">
          <p className="metric-label">
            {labels.sourceBreakdownTitle ?? "Field source breakdown"}
          </p>
          <div className="mt-3 overflow-hidden rounded-xl border border-base-300/70">
            {fieldRows.map((row) => {
              const editable = row.field != null && editablePolicy != null;
              const activeOverride =
                row.field != null && row.source === "account";
              const expanded =
                row.field != null && expandedFields.includes(row.field);
              const error =
                row.field != null
                  ? editablePolicy?.errorByField?.[row.field]
                  : null;
              const busy = row.field != null && isBusy(row.field);
              return (
                <div
                  key={row.label}
                  className="border-b border-base-300/60 last:border-b-0"
                >
                  <div className="grid grid-cols-1 gap-1 px-3 py-2.5 text-sm sm:grid-cols-[9rem_minmax(0,1fr)_minmax(5rem,auto)_2rem] sm:items-center sm:gap-3">
                    <span className="font-medium text-base-content/80">
                      {row.label}
                    </span>
                    {row.valueBadges ? (
                      <ValueBadgeList
                        field={row.field}
                        values={row.valueBadges}
                        labels={labels}
                      />
                    ) : (
                      <ValueBadge
                        field={row.field}
                        value={row.value}
                        labels={labels}
                      />
                    )}
                    <Badge
                      className="w-fit sm:justify-self-end"
                      variant={sourceVariant(row.source)}
                    >
                      {sourceLabel(row.source, labels)}
                    </Badge>
                    {editable && row.field ? (
                      <Button
                        type="button"
                        size="icon"
                        variant={
                          activeOverride || expanded ? "default" : "ghost"
                        }
                        className={cn(
                          "h-8 w-8 justify-self-start rounded-full sm:justify-self-end",
                          activeOverride || expanded
                            ? "text-primary-content"
                            : "text-base-content/65",
                        )}
                        disabled={busy}
                        aria-pressed={activeOverride || expanded}
                        aria-label={`${activeOverride ? (labels.overrideClear ?? "Clear override") : (labels.overrideEdit ?? "Edit override")}: ${row.label}`}
                        onClick={() =>
                          toggleExpanded(row.field, row.clearPayload)
                        }
                      >
                        <AppIcon
                          name={
                            busy
                              ? "loading"
                              : activeOverride || expanded
                                ? "check-decagram-outline"
                                : "pencil-outline"
                          }
                          className={cn("h-4 w-4", busy ? "animate-spin" : "")}
                          aria-hidden
                        />
                      </Button>
                    ) : (
                      <span aria-hidden />
                    )}
                  </div>
                  {expanded && row.field ? (
                    <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                      <div className="grid grid-cols-1 gap-y-2 sm:grid-cols-[9rem_minmax(0,1fr)_minmax(5rem,auto)_2rem] sm:items-center sm:gap-x-3">
                        <p className="text-sm font-semibold text-base-content">
                          {row.label}
                        </p>
                        <div className="min-w-0 sm:col-span-3">
                          {row.editor}
                        </div>
                        {busy ? (
                          <p className="text-xs text-base-content/60 sm:col-start-2 sm:col-span-3">
                            {labels.overrideSaving ?? "Saving..."}
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
                    <p className="px-3 pb-2 text-xs font-medium text-error">
                      {error}
                    </p>
                  ) : null}
                </div>
              );
            })}
            {proxyBindings ? (
              <div className="border-b border-base-300/60 last:border-b-0">
                <div className="grid grid-cols-1 gap-1 px-3 py-2.5 text-sm sm:grid-cols-[9rem_minmax(0,1fr)_minmax(5rem,auto)_2rem] sm:items-center sm:gap-3">
                  <span className="font-medium text-base-content/80">
                    {labels.fieldProxyBindings ?? proxyBindings.labels.field}
                  </span>
                  <ProxyBindingChips
                    items={proxyBindings.items}
                    labels={proxyBindings.labels}
                    disabled={proxyBindings.busy || proxyBindings.disabled}
                  />
                  <Badge
                    className="w-fit sm:justify-self-end"
                    variant={sourceVariant(proxyBindingsSource)}
                  >
                    {sourceLabel(proxyBindingsSource, labels)}
                  </Badge>
                  <Button
                    type="button"
                    size="icon"
                    variant={proxyBindingsActiveOverride ? "default" : "ghost"}
                    className={cn(
                      "h-8 w-8 justify-self-start rounded-full sm:justify-self-end",
                      proxyBindingsActiveOverride
                        ? "text-primary-content"
                        : "text-base-content/65",
                    )}
                    disabled={proxyBindings.busy || proxyBindings.disabled}
                    aria-pressed={proxyBindingsActiveOverride}
                    aria-label={`${proxyBindingsActiveOverride ? (labels.overrideClear ?? "Clear override") : (labels.overrideEdit ?? "Edit override")}: ${proxyBindings.labels.field}`}
                    onClick={toggleProxyBindingsRow}
                  >
                    <AppIcon
                      name={
                        proxyBindings.busy
                          ? "loading"
                          : proxyBindingsActiveOverride
                            ? "check-decagram-outline"
                            : "pencil-outline"
                      }
                      className={cn(
                        "h-4 w-4",
                        proxyBindings.busy ? "animate-spin" : "",
                      )}
                      aria-hidden
                    />
                  </Button>
                </div>
                {proxyBindingsActiveOverride && proxyBindingsExpanded ? (
                  <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                    <div className="grid grid-cols-1 gap-y-3 sm:grid-cols-[9rem_minmax(0,1fr)_minmax(5rem,auto)_2rem] sm:items-start sm:gap-x-3">
                      <p className="text-sm font-semibold text-base-content">
                        {proxyBindings.labels.field}
                      </p>
                      <div className="min-w-0 space-y-3 sm:col-span-3">
                        <ProxyBindingMultiSelectTrigger
                          items={proxyBindings.items}
                          labels={proxyBindings.labels}
                          disabled={
                            proxyBindings.busy || proxyBindings.disabled
                          }
                          onOpen={proxyBindings.onEdit}
                          onRemove={proxyBindings.onRemove}
                        />
                        <p className="text-xs leading-5 text-base-content/65">
                          {proxyBindings.labels.hint}
                        </p>
                      </div>
                    </div>
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>

        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-3">
          <p className="metric-label">
            {labels.timeoutSectionTitle ?? "Request path timeouts"}
          </p>
          <div className="mt-3 overflow-hidden rounded-xl border border-base-300/70">
            {timeoutRows.map((row) => {
              const activeOverride = row.source === "account";
              const expanded = expandedFields.includes(row.field);
              const busy = isBusy(row.field);
              const error = editablePolicy?.errorByField?.[row.field] ?? null;
              return (
                <div
                  key={row.key}
                  className="border-b border-base-300/60 last:border-b-0"
                >
                  <div className="grid grid-cols-1 gap-1 px-3 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_5rem_11rem_2rem] sm:items-center sm:gap-3">
                    <span className="min-w-0 font-medium text-base-content/80">
                      {row.label}
                    </span>
                    <ValueBadge
                      field={row.field}
                      value={row.value}
                      labels={labels}
                    />
                    <div className="min-w-0 flex flex-wrap items-center gap-2">
                      <span className="text-xs text-base-content/65">
                        {activeOverride
                          ? (labels.timeoutOverrideValue ?? "Account override")
                          : (labels.timeoutInheritedValue ?? "Inherited")}
                      </span>
                      <Badge
                        className="w-fit"
                        variant={sourceVariant(row.source)}
                      >
                        {sourceLabel(row.source, labels)}
                      </Badge>
                    </div>
                    {isEditable ? (
                      <Button
                        type="button"
                        size="icon"
                        variant={
                          activeOverride || expanded ? "default" : "ghost"
                        }
                        className={cn(
                          "h-8 w-8 justify-self-start rounded-full sm:justify-self-end",
                          activeOverride || expanded
                            ? "text-primary-content"
                            : "text-base-content/65",
                        )}
                        disabled={busy}
                        aria-pressed={activeOverride || expanded}
                        aria-label={`${activeOverride ? (labels.overrideClear ?? "Clear override") : (labels.overrideEdit ?? "Edit override")}: ${row.label}`}
                        onClick={() =>
                          toggleExpanded(row.field, row.clearPayload)
                        }
                      >
                        <AppIcon
                          name={
                            busy
                              ? "loading"
                              : activeOverride || expanded
                                ? "check-decagram-outline"
                                : "pencil-outline"
                          }
                          className={cn("h-4 w-4", busy ? "animate-spin" : "")}
                          aria-hidden
                        />
                      </Button>
                    ) : (
                      <span aria-hidden />
                    )}
                  </div>
                  {expanded ? (
                    <div className="border-t border-base-300/50 bg-base-100/55 px-3 py-3">
                      <div className="grid grid-cols-1 gap-y-2 sm:grid-cols-[minmax(0,1fr)_5rem_11rem_2rem] sm:items-center sm:gap-x-3">
                        <p className="min-w-0 text-sm font-semibold text-base-content">
                          {row.label}
                        </p>
                        <div className="min-w-0 sm:col-span-3">
                          <Input
                            name={row.key}
                            type="number"
                            min="1"
                            step="1"
                            defaultValue={String(timeoutValues[row.key])}
                            disabled={busy}
                            className="h-11 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                            onBlur={(
                              event: React.FocusEvent<HTMLInputElement>,
                            ) => {
                              const parsed = event.currentTarget.value.trim();
                              if (!parsed || !editablePolicy) return;
                              void editablePolicy.onChange(row.field, {
                                timeouts: {
                                  [row.key]: Number(parsed),
                                },
                              });
                            }}
                          />
                        </div>
                        {busy ? (
                          <p className="text-xs text-base-content/60 sm:col-start-2 sm:col-span-3">
                            {labels.overrideSaving ?? "Saving..."}
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
                    <p className="px-3 pb-2 text-xs font-medium text-error">
                      {error}
                    </p>
                  ) : null}
                </div>
              );
            })}
          </div>
          {inlineTimeoutBusy ? (
            <p className="mt-3 text-xs text-base-content/60">
              {labels.overrideSaving ?? "Saving..."}
            </p>
          ) : null}
        </div>

        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-3">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
            <div>
              <p className="metric-label">
                {labels.statusChangeReasonSectionTitle ??
                  "Status change trigger reasons"}
              </p>
              {labels.statusChangeReasonSectionHint ? (
                <p className="mt-1 text-xs leading-5 text-base-content/65">
                  {labels.statusChangeReasonSectionHint}
                </p>
              ) : null}
            </div>
            <div className="flex items-center gap-2 self-start">
              {editablePolicy && statusChangeReasonHasAccountOverrides ? (
                <Button
                  type="button"
                  variant="ghost"
                  className="h-8 rounded-full border border-base-300/80 bg-base-100/92 px-3 text-xs font-semibold text-base-content/72 shadow-none hover:bg-base-100 hover:text-base-content"
                  disabled={statusChangeReasonResetBusy}
                  aria-label={
                    labels.statusChangeReasonResetAction ??
                    "Reset status change trigger reasons"
                  }
                  onClick={() => {
                    if (!statusChangeReasonResetPayload) return;
                    changeField(
                      "statusChangeReasons",
                      statusChangeReasonResetPayload,
                    );
                  }}
                >
                  {statusChangeReasonResetBusy ? (
                    <AppIcon
                      name="loading"
                      className="mr-1 h-3.5 w-3.5 animate-spin"
                      aria-hidden
                    />
                  ) : null}
                  {labels.statusChangeReasonResetAction ?? "Reset"}
                </Button>
              ) : null}
              <Badge variant="secondary" className="w-fit">
                {labels.statusChangeReasonSummary?.(
                  totalEnabledStatusChangeReasons,
                  STATUS_CHANGE_REASON_CODES.length,
                ) ??
                  `${totalEnabledStatusChangeReasons}/${STATUS_CHANGE_REASON_CODES.length}`}
              </Badge>
            </div>
          </div>
          <div className="mt-3 grid gap-2 md:auto-rows-fr md:grid-cols-2 lg:grid-cols-4 xl:grid-cols-5">
            {statusChangeReasonRows.map((row) => {
              const busy = isBusy(row.field);
              const error = editablePolicy?.errorByField?.[row.field] ?? null;
              return (
                <div key={row.reason} className="flex h-full flex-col gap-1.5">
                  <div className="flex flex-1">
                    <StatusChangeToggleButton
                      title={row.label}
                      iconName={statusChangeReasonIconName(row.reason)}
                      pressed={row.enabled}
                      disabled={editablePolicy ? busy : undefined}
                      interactive={editablePolicy != null}
                      activeLabel={labels.statusChangeReasonToggleEnabled}
                      inactiveLabel={labels.statusChangeReasonToggleDisabled}
                      onPressedChange={(checked) =>
                        changeField(row.field, {
                          statusChangeReasons: {
                            [row.reason]: checked,
                          },
                        })
                      }
                      ariaLabel={row.label}
                      className="h-full min-h-[4rem] flex-1"
                    />
                  </div>
                  {busy ? (
                    <p className="text-xs text-base-content/60">
                      {labels.overrideSaving ?? "Saving..."}
                    </p>
                  ) : null}
                  {error ? (
                    <p className="text-xs font-medium text-error">{error}</p>
                  ) : null}
                </div>
              );
            })}
          </div>
          {statusChangeReasonSectionError ? (
            <p className="mt-3 text-xs font-medium text-error">
              {statusChangeReasonSectionError}
            </p>
          ) : null}
        </div>

        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-3">
          <p className="metric-label">{labels.sourceTags}</p>
          <div className="mt-3 flex flex-wrap gap-2">
            {resolvedRule.sourceTagNames.length === 0 ? (
              <span className="text-sm text-base-content/60">
                {labels.noTags}
              </span>
            ) : (
              resolvedRule.sourceTagNames.map((name) => (
                <Badge key={name} variant="secondary">
                  {name}
                </Badge>
              ))
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

function fieldToSource(
  field: EditablePolicyField,
  sources: FieldSourceMap,
  timeoutSources: EffectiveRoutingTimeoutFieldSources,
  reasonSources: StatusChangeReasonFieldSources,
): EffectiveRoutingRuleSource {
  const reason = statusChangeReasonFromFieldKey(field);
  if (reason) {
    return reasonSources[reason];
  }
  switch (field) {
    case "allowCutOut":
      return sources.allowCutOut;
    case "allowCutIn":
      return sources.allowCutIn;
    case "priorityTier":
      return sources.priorityTier;
    case "fastModeRewriteMode":
      return sources.fastModeRewriteMode;
    case "imageToolRewriteMode":
      return sources.imageToolRewriteMode ?? "root";
    case "concurrencyLimit":
      return sources.concurrencyLimit;
    case "upstream429Retry":
      return sources.upstream429Retry;
    case "availableModels":
      return sources.availableModels ?? "root";
    case "timeoutResponsesFirstByte":
      return timeoutSources.responsesFirstByteTimeoutSecs;
    case "timeoutCompactFirstByte":
      return timeoutSources.compactFirstByteTimeoutSecs;
    case "timeoutResponsesStream":
      return timeoutSources.responsesStreamTimeoutSecs;
    case "timeoutCompactStream":
      return timeoutSources.compactStreamTimeoutSecs;
    case "statusChangeReasons":
    case "proxyBindings":
      return "account";
  }
  return "root";
}

interface ConcurrencyInlineEditorProps {
  value: number;
  disabled?: boolean;
  currentLabel: string;
  unlimitedLabel: string;
  onChange: (value: number) => void;
}

function ConcurrencyInlineEditor({
  value,
  disabled,
  currentLabel,
  unlimitedLabel,
  onChange,
}: ConcurrencyInlineEditorProps) {
  const sliderValue = apiConcurrencyLimitToSliderValue(value);
  const displayValue = formatConcurrencyLimitValue(value, unlimitedLabel);
  return (
    <div className="min-w-[16rem] space-y-2">
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs font-semibold uppercase tracking-[0.12em] text-base-content/55">
          {currentLabel}
        </span>
        <span className="rounded-full border border-base-300/80 bg-base-200/80 px-2.5 py-1 text-sm font-semibold text-base-content">
          {displayValue}
        </span>
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
        onChange={(event) =>
          onChange(sliderConcurrencyLimitToApiValue(Number(event.target.value)))
        }
        className="h-2 w-full cursor-pointer appearance-none rounded-full bg-base-300 accent-primary disabled:cursor-not-allowed disabled:opacity-60"
      />
      <div className="flex items-center justify-between text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/45">
        <span>{CONCURRENCY_LIMIT_MIN}</span>
        <span>{CONCURRENCY_LIMIT_MAX}</span>
        <span title={unlimitedLabel}>∞</span>
      </div>
    </div>
  );
}

interface RetryInlineEditorProps {
  retries: number;
  disabled?: boolean;
  labels: EffectiveRoutingRuleCardProps["labels"];
  onChange: (count: number) => void;
}

function RetryInlineEditor({
  retries,
  disabled,
  labels,
  onChange,
}: RetryInlineEditorProps) {
  const value = Math.min(5, Math.max(0, Math.trunc(retries)));
  return (
    <PolicyInlineOptionGroup<number>
      ariaLabel={labels.fieldUpstream429 ?? "Upstream 429 retry"}
      value={value}
      disabled={disabled}
      options={[0, 1, 2, 3, 4, 5].map((option) => ({
        value: option,
        label: formatUpstream429RetryCount(option, labels),
      }))}
      onChange={onChange}
    />
  );
}

interface AvailableModelsEditorProps {
  value: string[];
  options: string[];
  inputValue: string;
  emptyValueLabel: string;
  disabled?: boolean;
  labels: EffectiveRoutingRuleCardProps["labels"];
  onInputChange: (value: string) => void;
  onAdd: (value: string) => void;
  onChange: (value: string[]) => void;
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
  const trimmedInput = inputValue.trim();
  const canAdd = trimmedInput.length > 0 && !value.includes(trimmedInput);
  const [open, setOpen] = useState(false);
  const selectedValueSet = useMemo(() => new Set(value), [value]);
  const availableOptions = useMemo(
    () =>
      options.filter(
        (option, index) => option.trim() && options.indexOf(option) === index,
      ),
    [options],
  );
  const filteredOptions = useMemo(() => {
    if (!trimmedInput) return availableOptions;
    const query = trimmedInput.toLocaleLowerCase();
    return availableOptions.filter((option) =>
      option.toLocaleLowerCase().includes(query),
    );
  }, [availableOptions, trimmedInput]);

  const commitCustomValue = () => {
    if (!canAdd) return;
    onAdd(trimmedInput);
    setOpen(false);
  };

  const triggerTitle = value.length > 0 ? value.join(", ") : emptyValueLabel;

  return (
    <div className="min-w-[18rem]">
      <Popover
        open={disabled ? false : open}
        onOpenChange={(nextOpen) => {
          if (disabled) {
            setOpen(false);
            return;
          }
          setOpen(nextOpen);
          if (!nextOpen) {
            onInputChange("");
          }
        }}
      >
        <PopoverTrigger asChild>
          <button
            type="button"
            role="combobox"
            aria-expanded={open}
            aria-label={labels.fieldAvailableModels ?? "Available models"}
            disabled={disabled}
            title={triggerTitle}
            className={cn(
              "flex w-full items-center gap-3 rounded-xl border border-base-300 bg-base-100 px-3 py-2.5 text-left shadow-sm transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
              "hover:border-primary/35",
              disabled && "cursor-not-allowed opacity-60",
            )}
          >
            <AppIcon
              name="tag-outline"
              className="mt-0.5 h-4 w-4 shrink-0 text-base-content/55"
              aria-hidden
            />
            <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
              {value.length > 0 ? (
                value.map((model) => (
                  <Badge
                    key={model}
                    variant="secondary"
                    className="max-w-full rounded-full border border-primary/20 bg-primary/10 px-2.5 py-1 text-primary"
                  >
                    <span className="truncate">
                      {labels.availableModelsCustomLabel?.(model) ?? model}
                    </span>
                  </Badge>
                ))
              ) : (
                <span className="text-sm text-base-content/55">
                  {emptyValueLabel}
                </span>
              )}
            </span>
            <AppIcon
              name="chevron-down"
              className="h-4 w-4 shrink-0 text-base-content/45"
              aria-hidden
            />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-[var(--radix-popover-trigger-width)] p-0"
        >
          <Command shouldFilter={false}>
            <CommandInput
              value={inputValue}
              placeholder={
                labels.availableModelsPlaceholder ??
                labels.availableModelsAddCustom ??
                "Add model"
              }
              onValueChange={onInputChange}
            />
            <CommandList>
              {canAdd ? (
                <>
                  <CommandGroup>
                    <CommandItem
                      value={trimmedInput}
                      onSelect={commitCustomValue}
                    >
                      <AppIcon
                        name="plus-circle-outline"
                        className="mr-2 h-4 w-4 text-primary"
                        aria-hidden
                      />
                      <span className="truncate">{trimmedInput}</span>
                    </CommandItem>
                  </CommandGroup>
                  <CommandSeparator />
                </>
              ) : null}
              {filteredOptions.length === 0 ? (
                <CommandEmpty>
                  {labels.availableModelsEmpty ?? "No matching models"}
                </CommandEmpty>
              ) : (
                <CommandGroup>
                  {filteredOptions.map((model) => {
                    const active = selectedValueSet.has(model);
                    return (
                      <CommandItem
                        key={model}
                        value={model}
                        disabled={disabled}
                        onSelect={() =>
                          onChange(
                            active
                              ? value.filter((item) => item !== model)
                              : [...value, model],
                          )
                        }
                      >
                        <AppIcon
                          name="check"
                          className={cn(
                            "mr-2 h-4 w-4 text-primary transition-opacity",
                            active ? "opacity-100" : "opacity-0",
                          )}
                          aria-hidden
                        />
                        <span className="truncate">
                          {labels.availableModelsCustomLabel?.(model) ?? model}
                        </span>
                      </CommandItem>
                    );
                  })}
                </CommandGroup>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  );
}
