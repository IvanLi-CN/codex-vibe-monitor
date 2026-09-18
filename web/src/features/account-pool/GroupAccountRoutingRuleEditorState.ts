import { useEffect, useMemo, useRef, useState } from "react";
import type {
  EffectiveRoutingTimeoutFieldSources,
  GroupAccountRoutingRule,
  PoolRoutingTimeoutSettings,
  UpdateGroupAccountRoutingRulePayload,
} from "../../lib/api";
import { parseRoutingTimeoutOverrideDraftWithEnabledState } from "../../lib/poolRoutingTimeouts";
import {
  buildDraft,
  buildDraftResetKey,
  buildPayload,
  type GroupAccountRoutingRuleDraft,
  type GroupAccountRoutingRuleLabels,
} from "./GroupAccountRoutingRuleDialog.model";

export interface GroupAccountRoutingRuleEditorStateOptions {
  open: boolean;
  rule?: GroupAccountRoutingRule | null;
  changedFieldsOnly: boolean;
  effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
  timeoutOverrideSource: "group" | "account";
  labels: GroupAccountRoutingRuleLabels;
  onPayloadChange?: (payload: UpdateGroupAccountRoutingRulePayload | null) => void;
}

export function useGroupAccountRoutingRuleEditorState({
  open,
  rule,
  changedFieldsOnly,
  effectiveTimeouts,
  timeoutFieldSources,
  timeoutOverrideSource,
  labels,
  onPayloadChange,
}: GroupAccountRoutingRuleEditorStateOptions) {
  const [draft, setDraft] = useState<GroupAccountRoutingRuleDraft>(() =>
    buildDraft(rule, {
      changedFieldsOnly,
      effectiveTimeouts,
      timeoutFieldSources,
      timeoutOverrideSource,
    }),
  );
  const [baseRule, setBaseRule] = useState<GroupAccountRoutingRule | null>(() => rule ?? null);
  const previousOpenRef = useRef(open);
  const activeResetKeyRef = useRef<string | null>(open ? buildDraftResetKey(rule) : null);
  const resetKey = buildDraftResetKey(rule);
  const timeoutFieldLabels = useMemo(
    () => ({
      responsesFirstByteTimeoutSecs: labels.timeoutResponsesFirstByte,
      compactFirstByteTimeoutSecs: labels.timeoutCompactFirstByte,
      imageFirstByteTimeoutSecs: labels.timeoutImageFirstByte,
      responsesStreamTimeoutSecs: labels.timeoutResponsesStream,
      compactStreamTimeoutSecs: labels.timeoutCompactStream,
    }),
    [labels],
  );

  useEffect(() => {
    const wasOpen = previousOpenRef.current;
    previousOpenRef.current = open;
    if (!open) {
      activeResetKeyRef.current = null;
      return;
    }
    if (wasOpen && activeResetKeyRef.current === resetKey) return;
    const nextBaseRule = rule ?? null;
    activeResetKeyRef.current = resetKey;
    setBaseRule(nextBaseRule);
    setDraft(
      buildDraft(nextBaseRule, {
        changedFieldsOnly,
        effectiveTimeouts,
        timeoutFieldSources,
        timeoutOverrideSource,
      }),
    );
  }, [
    changedFieldsOnly,
    effectiveTimeouts,
    open,
    resetKey,
    rule,
    timeoutFieldSources,
    timeoutOverrideSource,
  ]);

  const parsedTimeouts = parseRoutingTimeoutOverrideDraftWithEnabledState(
    draft.timeoutOverrides,
    draft.timeoutOverrideEnabledFields,
    timeoutFieldLabels,
  );
  const timeoutValidationError = parsedTimeouts.ok ? null : parsedTimeouts.error;
  const payload = useMemo(
    () =>
      buildPayload(draft, {
        changedFieldsOnly,
        baseRule,
        effectiveTimeouts,
        timeoutFieldSources,
        timeoutFieldLabels,
        timeoutOverrideSource,
      }),
    [
      baseRule,
      changedFieldsOnly,
      draft,
      effectiveTimeouts,
      timeoutFieldLabels,
      timeoutFieldSources,
      timeoutOverrideSource,
    ],
  );

  useEffect(() => {
    onPayloadChange?.(payload);
  }, [onPayloadChange, payload]);

  return { draft, setDraft, payload, timeoutValidationError };
}
