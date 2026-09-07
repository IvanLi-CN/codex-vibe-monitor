import type {
  EffectiveRoutingRule,
  EffectiveRoutingRuleSource,
  UpdateGroupAccountRoutingRulePayload,
} from "./api";
import { applyRoutingTimeoutOverridePatch } from "./poolRoutingTimeouts";
import {
  buildDefaultStatusChangeReasons,
  STATUS_CHANGE_REASON_CODES,
} from "./upstreamAccountStatusChangeReasons";

const ROUTING_RULE_FIELDS = [
  "allowCutOut",
  "allowCutIn",
  "priorityTier",
  "fastModeRewriteMode",
  "imageToolRewriteMode",
  "codexImagegenRewriteMode",
  "requestCompressionAlgorithm",
  "concurrencyLimit",
  "upstream429RetryEnabled",
  "upstream429MaxRetries",
  "availableModels",
  "availableModelsMode",
] as const satisfies ReadonlyArray<keyof UpdateGroupAccountRoutingRulePayload>;

type RoutingRuleField = (typeof ROUTING_RULE_FIELDS)[number];

function setFieldSource(
  fieldSources: NonNullable<EffectiveRoutingRule["fieldSources"]>,
  field: RoutingRuleField,
  source: EffectiveRoutingRuleSource,
) {
  const sourceKey = field === "upstream429RetryEnabled" ? "upstream429Retry" : field;
  if (sourceKey in fieldSources) {
    fieldSources[sourceKey as keyof typeof fieldSources] = source;
  }
}

/** Apply a known local patch to an effective rule for optimistic rendering. */
export function applyRoutingRulePatchToEffectiveRule(
  rule: EffectiveRoutingRule,
  patch: UpdateGroupAccountRoutingRulePayload,
  source: EffectiveRoutingRuleSource = "account",
): EffectiveRoutingRule {
  const nextFieldSources = {
    allowCutOut: rule.fieldSources?.allowCutOut ?? "root",
    allowCutIn: rule.fieldSources?.allowCutIn ?? "root",
    priorityTier: rule.fieldSources?.priorityTier ?? "root",
    fastModeRewriteMode: rule.fieldSources?.fastModeRewriteMode ?? "root",
    imageToolRewriteMode: rule.fieldSources?.imageToolRewriteMode ?? "root",
    codexImagegenRewriteMode: rule.fieldSources?.codexImagegenRewriteMode ?? "root",
    requestCompressionAlgorithm: rule.fieldSources?.requestCompressionAlgorithm ?? "root",
    concurrencyLimit: rule.fieldSources?.concurrencyLimit ?? "root",
    upstream429Retry: rule.fieldSources?.upstream429Retry ?? "root",
    availableModels: rule.fieldSources?.availableModels ?? "root",
    availableModelsMode: rule.fieldSources?.availableModelsMode ?? "root",
    systemDeniedModels: rule.fieldSources?.systemDeniedModels ?? "root",
  };
  const next: EffectiveRoutingRule = {
    ...rule,
    fieldSources: nextFieldSources,
  };

  for (const field of ROUTING_RULE_FIELDS) {
    if (!Object.hasOwn(patch, field)) continue;
    const value = patch[field];
    if (value != null) {
      (next as Record<RoutingRuleField, unknown>)[field] = Array.isArray(value)
        ? [...value]
        : value;
    }
    setFieldSource(nextFieldSources, field, value == null ? "root" : source);
  }

  if (patch.statusChangeReasons) {
    const statusChangeReasons = {
      ...buildDefaultStatusChangeReasons(),
      ...(rule.statusChangeReasons ?? {}),
    };
    const statusChangeReasonFieldSources = {
      ...Object.fromEntries(
        STATUS_CHANGE_REASON_CODES.map((reason) => [
          reason,
          rule.statusChangeReasonFieldSources?.[reason] ?? "root",
        ]),
      ),
    } as NonNullable<EffectiveRoutingRule["statusChangeReasonFieldSources"]>;
    for (const reason of STATUS_CHANGE_REASON_CODES) {
      if (!Object.hasOwn(patch.statusChangeReasons, reason)) continue;
      const value = patch.statusChangeReasons[reason];
      if (value != null) statusChangeReasons[reason] = value;
      statusChangeReasonFieldSources[reason] = value == null ? "root" : source;
    }
    next.statusChangeReasons = statusChangeReasons;
    next.statusChangeReasonFieldSources = statusChangeReasonFieldSources;
  }

  if (patch.timeouts) {
    next.timeouts = {
      ...(rule.timeouts ?? {}),
      ...(applyRoutingTimeoutOverridePatch(rule.timeouts, patch.timeouts) ?? {}),
    } as EffectiveRoutingRule["timeouts"];
    const nextTimeoutSources = {
      ...(rule.timeoutFieldSources ?? {}),
    };
    for (const key of Object.keys(patch.timeouts) as Array<
      keyof NonNullable<typeof patch.timeouts>
    >) {
      nextTimeoutSources[key] = patch.timeouts[key] == null ? "root" : source;
    }
    next.timeoutFieldSources = nextTimeoutSources as EffectiveRoutingRule["timeoutFieldSources"];
  }

  return next;
}
