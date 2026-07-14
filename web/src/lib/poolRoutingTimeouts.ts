import type {
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingTimeoutSettings,
} from "./api";

export type RoutingTimeoutFieldKey = keyof PoolRoutingTimeoutSettings;

export type RoutingTimeoutOverrideDraft = Partial<Record<RoutingTimeoutFieldKey, string>>;

export type RoutingTimeoutOverridePatch = Partial<Record<RoutingTimeoutFieldKey, number | null>>;

export type RoutingTimeoutOverrideEnabledState = Partial<Record<RoutingTimeoutFieldKey, boolean>>;

export const ROUTING_TIMEOUT_FIELD_ORDER: RoutingTimeoutFieldKey[] = [
  "responsesFirstByteTimeoutSecs",
  "compactFirstByteTimeoutSecs",
  "imageFirstByteTimeoutSecs",
  "responsesStreamTimeoutSecs",
  "compactStreamTimeoutSecs",
];

export const DEFAULT_TIMEOUT_FIELD_SOURCES: EffectiveRoutingTimeoutFieldSources = {
  responsesFirstByteTimeoutSecs: "root",
  compactFirstByteTimeoutSecs: "root",
  imageFirstByteTimeoutSecs: "root",
  responsesStreamTimeoutSecs: "root",
  compactStreamTimeoutSecs: "root",
};

export function buildRoutingTimeoutOverrideDraft(
  override?: Partial<PoolRoutingTimeoutSettings> | null,
): RoutingTimeoutOverrideDraft {
  return {
    responsesFirstByteTimeoutSecs:
      override?.responsesFirstByteTimeoutSecs != null
        ? String(override.responsesFirstByteTimeoutSecs)
        : "",
    compactFirstByteTimeoutSecs:
      override?.compactFirstByteTimeoutSecs != null
        ? String(override.compactFirstByteTimeoutSecs)
        : "",
    imageFirstByteTimeoutSecs:
      override?.imageFirstByteTimeoutSecs != null ? String(override.imageFirstByteTimeoutSecs) : "",
    responsesStreamTimeoutSecs:
      override?.responsesStreamTimeoutSecs != null
        ? String(override.responsesStreamTimeoutSecs)
        : "",
    compactStreamTimeoutSecs:
      override?.compactStreamTimeoutSecs != null ? String(override.compactStreamTimeoutSecs) : "",
  };
}

export function buildRoutingTimeoutOverrideDraftForSource(
  effective: Partial<PoolRoutingTimeoutSettings> | null | undefined,
  sources: EffectiveRoutingTimeoutFieldSources | null | undefined,
  targetSource: EffectiveRoutingRuleSource,
): RoutingTimeoutOverrideDraft {
  return Object.fromEntries(
    ROUTING_TIMEOUT_FIELD_ORDER.map((key) => [
      key,
      getRoutingTimeoutFieldSource(sources, key) === targetSource && effective?.[key] != null
        ? String(effective[key])
        : "",
    ]),
  ) as RoutingTimeoutOverrideDraft;
}

export function trimRoutingTimeoutOverrideDraft(
  draft: RoutingTimeoutOverrideDraft,
): RoutingTimeoutOverrideDraft {
  const next: RoutingTimeoutOverrideDraft = {};
  for (const key of ROUTING_TIMEOUT_FIELD_ORDER) {
    next[key] = draft[key]?.trim() ?? "";
  }
  return next;
}

export function buildRoutingTimeoutOverrideEnabledState(
  draft: RoutingTimeoutOverrideDraft,
): RoutingTimeoutOverrideEnabledState {
  return Object.fromEntries(
    ROUTING_TIMEOUT_FIELD_ORDER.map((key) => [key, (draft[key] ?? "").trim() !== ""]),
  ) as RoutingTimeoutOverrideEnabledState;
}

export function buildRoutingTimeoutOverrideEnabledStateForSource(
  sources: EffectiveRoutingTimeoutFieldSources | null | undefined,
  targetSource: EffectiveRoutingRuleSource,
): RoutingTimeoutOverrideEnabledState {
  return Object.fromEntries(
    ROUTING_TIMEOUT_FIELD_ORDER.map((key) => [
      key,
      getRoutingTimeoutFieldSource(sources, key) === targetSource,
    ]),
  ) as RoutingTimeoutOverrideEnabledState;
}

export function routingTimeoutOverrideDraftHasAnyValue(
  draft: RoutingTimeoutOverrideDraft,
): boolean {
  return ROUTING_TIMEOUT_FIELD_ORDER.some((key) => (draft[key] ?? "").trim() !== "");
}

export function parseRoutingTimeoutOverrideDraft(
  draft: RoutingTimeoutOverrideDraft,
  labels: Record<RoutingTimeoutFieldKey, string>,
): { ok: true; patch: RoutingTimeoutOverridePatch } | { ok: false; error: string } {
  const trimmed = trimRoutingTimeoutOverrideDraft(draft);
  const patch: RoutingTimeoutOverridePatch = {};
  for (const key of ROUTING_TIMEOUT_FIELD_ORDER) {
    const raw = trimmed[key] ?? "";
    if (!raw) continue;
    if (!/^[1-9]\d*$/.test(raw)) {
      return { ok: false, error: `${labels[key]} must be a positive integer.` };
    }
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed)) {
      return { ok: false, error: `${labels[key]} must be a positive integer.` };
    }
    patch[key] = parsed;
  }
  return { ok: true, patch };
}

export function parseRoutingTimeoutOverrideDraftWithEnabledState(
  draft: RoutingTimeoutOverrideDraft,
  enabled: RoutingTimeoutOverrideEnabledState,
  labels: Record<RoutingTimeoutFieldKey, string>,
): { ok: true; patch: RoutingTimeoutOverridePatch } | { ok: false; error: string } {
  const trimmed = trimRoutingTimeoutOverrideDraft(draft);
  const patch: RoutingTimeoutOverridePatch = {};
  for (const key of ROUTING_TIMEOUT_FIELD_ORDER) {
    const isEnabled = enabled[key] === true;
    const raw = trimmed[key] ?? "";
    if (!isEnabled) {
      patch[key] = null;
      continue;
    }
    if (!raw) {
      return { ok: false, error: `${labels[key]} must be a positive integer.` };
    }
    if (!/^[1-9]\d*$/.test(raw)) {
      return { ok: false, error: `${labels[key]} must be a positive integer.` };
    }
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed)) {
      return { ok: false, error: `${labels[key]} must be a positive integer.` };
    }
    patch[key] = parsed;
  }
  return { ok: true, patch };
}

export function diffRoutingTimeoutOverrideDraft(
  baseDraft: RoutingTimeoutOverrideDraft,
  draft: RoutingTimeoutOverrideDraft,
  labels: Record<RoutingTimeoutFieldKey, string>,
):
  | { ok: true; patch: RoutingTimeoutOverridePatch; changed: boolean }
  | { ok: false; error: string } {
  const parsed = parseRoutingTimeoutOverrideDraft(draft, labels);
  if (!parsed.ok) {
    return parsed;
  }
  const trimmedBase = trimRoutingTimeoutOverrideDraft(baseDraft);
  const trimmedDraft = trimRoutingTimeoutOverrideDraft(draft);
  const patch: RoutingTimeoutOverridePatch = {};
  for (const key of ROUTING_TIMEOUT_FIELD_ORDER) {
    if ((trimmedDraft[key] ?? "") === (trimmedBase[key] ?? "")) {
      continue;
    }
    patch[key] = trimmedDraft[key] ? (parsed.patch[key] ?? null) : null;
  }
  return {
    ok: true,
    patch,
    changed: Object.keys(patch).length > 0,
  };
}

export function diffRoutingTimeoutOverrideDraftWithEnabledState(
  baseDraft: RoutingTimeoutOverrideDraft,
  baseEnabled: RoutingTimeoutOverrideEnabledState,
  draft: RoutingTimeoutOverrideDraft,
  enabled: RoutingTimeoutOverrideEnabledState,
  labels: Record<RoutingTimeoutFieldKey, string>,
):
  | { ok: true; patch: RoutingTimeoutOverridePatch; changed: boolean }
  | { ok: false; error: string } {
  const parsed = parseRoutingTimeoutOverrideDraftWithEnabledState(draft, enabled, labels);
  if (!parsed.ok) {
    return parsed;
  }
  const trimmedBase = trimRoutingTimeoutOverrideDraft(baseDraft);
  const trimmedDraft = trimRoutingTimeoutOverrideDraft(draft);
  const patch: RoutingTimeoutOverridePatch = {};
  for (const key of ROUTING_TIMEOUT_FIELD_ORDER) {
    const nextEnabled = enabled[key] === true;
    const previousEnabled = baseEnabled[key] === true;
    const nextValue = nextEnabled ? (trimmedDraft[key] ?? "") : "";
    const previousValue = previousEnabled ? (trimmedBase[key] ?? "") : "";
    if (nextEnabled === previousEnabled && nextValue === previousValue) {
      continue;
    }
    patch[key] = nextEnabled ? (parsed.patch[key] ?? null) : null;
  }
  return {
    ok: true,
    patch,
    changed: Object.keys(patch).length > 0,
  };
}

export function applyRoutingTimeoutOverridePatch(
  base: Partial<PoolRoutingTimeoutSettings> | null | undefined,
  patch: RoutingTimeoutOverridePatch | null | undefined,
): Partial<PoolRoutingTimeoutSettings> | undefined {
  if (!patch) {
    return base ?? undefined;
  }
  const next: Partial<PoolRoutingTimeoutSettings> = { ...(base ?? {}) };
  for (const key of ROUTING_TIMEOUT_FIELD_ORDER) {
    if (!(key in patch)) {
      continue;
    }
    const value = patch[key];
    if (value == null) {
      delete next[key];
      continue;
    }
    next[key] = value;
  }
  return Object.keys(next).length > 0 ? next : undefined;
}

export function getRoutingTimeoutFieldSource(
  sources: EffectiveRoutingTimeoutFieldSources | null | undefined,
  key: RoutingTimeoutFieldKey,
): EffectiveRoutingRuleSource {
  return (sources ?? DEFAULT_TIMEOUT_FIELD_SOURCES)[key] ?? "root";
}

export function sourceTokenToUiLabel(
  source: EffectiveRoutingRuleSource,
  labels?: {
    root?: string;
    group?: string;
    account?: string;
    conversation?: string;
  },
): string {
  switch (source) {
    case "root":
      return labels?.root ?? "Global";
    case "group":
      return labels?.group ?? "Group";
    case "account":
      return labels?.account ?? "Account";
    case "conversation":
      return labels?.conversation ?? "Conversation";
    default:
      return source;
  }
}
