import { useCallback, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type {
  GroupAccountRoutingRule,
  PoolRoutingTimeoutSettings,
  EffectiveRoutingTimeoutFieldSources,
  UpdateGroupAccountRoutingRulePayload,
  UpdateUpstreamAccountGroupPayload,
} from "../../lib/api";
import {
  apiConcurrencyLimitToSliderValue,
  sliderConcurrencyLimitToApiValue,
} from "../../lib/concurrencyLimit";
import { applyRoutingTimeoutOverridePatch } from "../../lib/poolRoutingTimeouts";
import {
  STATUS_CHANGE_REASON_GROUPS,
  buildDefaultStatusChangeReasons,
  type StatusChangeReasonCode,
} from "../../lib/upstreamAccountStatusChangeReasons";
import { normalizeGroupName } from "../../lib/upstreamAccountGroups";
import { useTranslation } from "../../i18n";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import { useAvailableModelOptions } from "../../hooks/useAvailableModelOptions";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { GroupAccountRoutingRuleDialog } from "../../components/GroupAccountRoutingRuleDialog";
import { useGroupNoteCatalogAutoRefresh } from "./useGroupNoteCatalogAutoRefresh";

type GroupSettingsEditorState = {
  open: boolean;
  groupName: string;
  note: string;
  existing: boolean;
  accountCount: number;
  concurrencyLimit: number;
  boundProxyKeys: string[];
  nodeShuntEnabled: boolean;
  singleAccountRotationEnabled: boolean;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
  routingRule: GroupAccountRoutingRule;
  routingRuleDirty: boolean;
  policyEditorOpen: boolean;
  onSaved?: ((groupName: string) => void) | null;
  onDeleted?: ((groupName: string) => void) | null;
};

export type UpstreamAccountGroupSettingsSnapshot = {
  groupName: string;
  note?: string | null;
  existing?: boolean;
  accountCount?: number | null;
  concurrencyLimit?: number | null;
  boundProxyKeys?: string[];
  nodeShuntEnabled?: boolean;
  singleAccountRotationEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  routingRule?: GroupAccountRoutingRule;
  effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
};

const defaultRoutingRule: GroupAccountRoutingRule = {
  blockNewConversations: false,
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: "normal",
  fastModeRewriteMode: "keep_original",
  imageToolRewriteMode: "keep_original",
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
  availableModels: [],
  availableModelsDefined: false,
  statusChangeReasons: buildDefaultStatusChangeReasons(),
};

export function mergeRoutingRulePatch(
  base: GroupAccountRoutingRule,
  patch: UpdateGroupAccountRoutingRulePayload,
): GroupAccountRoutingRule {
  const nextStatusChangeReasons = {
    ...buildDefaultStatusChangeReasons(),
    ...(base.statusChangeReasons ?? {}),
  };
  if (patch.statusChangeReasons) {
    for (const group of STATUS_CHANGE_REASON_GROUPS) {
      for (const reason of group.reasonCodes) {
        if (
          !Object.prototype.hasOwnProperty.call(
            patch.statusChangeReasons,
            reason,
          )
        ) {
          continue;
        }
        const value = patch.statusChangeReasons[reason];
        nextStatusChangeReasons[reason] =
          typeof value === "boolean"
            ? value
            : buildDefaultStatusChangeReasons()[reason];
      }
    }
  }
  return {
    ...base,
    ...(patch.allowNewConversations == null
      ? {}
      : { blockNewConversations: !patch.allowNewConversations }),
    ...(patch.blockNewConversations == null
      ? {}
      : { blockNewConversations: patch.blockNewConversations }),
    ...(patch.allowCutOut == null ? {} : { allowCutOut: patch.allowCutOut }),
    ...(patch.allowCutIn == null ? {} : { allowCutIn: patch.allowCutIn }),
    ...(patch.priorityTier == null ? {} : { priorityTier: patch.priorityTier }),
    ...(patch.fastModeRewriteMode == null
      ? {}
      : { fastModeRewriteMode: patch.fastModeRewriteMode }),
    ...(patch.imageToolRewriteMode == null
      ? {}
      : { imageToolRewriteMode: patch.imageToolRewriteMode }),
    ...(patch.concurrencyLimit == null
      ? {}
      : { concurrencyLimit: patch.concurrencyLimit }),
    ...(patch.upstream429RetryEnabled == null
      ? {}
      : { upstream429RetryEnabled: patch.upstream429RetryEnabled }),
    ...(patch.upstream429MaxRetries == null
      ? {}
      : { upstream429MaxRetries: patch.upstream429MaxRetries }),
    ...(patch.availableModels == null
      ? {}
      : { availableModels: patch.availableModels }),
    ...(patch.statusChangeReasons == null
      ? {}
      : { statusChangeReasons: nextStatusChangeReasons }),
    ...(patch.timeouts == null
      ? {}
      : {
          timeouts: applyRoutingTimeoutOverridePatch(
            base.timeouts,
            patch.timeouts,
          ),
        }),
  };
}

function createInitialEditorState(): GroupSettingsEditorState {
  return {
    open: false,
    groupName: "",
    note: "",
    existing: false,
    accountCount: 0,
    concurrencyLimit: apiConcurrencyLimitToSliderValue(0),
    boundProxyKeys: [],
    nodeShuntEnabled: false,
    singleAccountRotationEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    routingRule: defaultRoutingRule,
    routingRuleDirty: false,
    policyEditorOpen: false,
    onSaved: null,
    onDeleted: null,
  };
}

function normalizeBoundProxyKeys(values?: string[]) {
  if (!Array.isArray(values)) return [];
  return Array.from(
    new Set(
      values.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  );
}

function normalizeUpstream429MaxRetries(value?: number | null) {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value ?? 0)));
}

function normalizeEnabledUpstream429MaxRetries(value?: number | null) {
  return Math.max(1, normalizeUpstream429MaxRetries(value) || 1);
}

interface UseUpstreamAccountGroupSettingsDialogOptions {
  writesEnabled: boolean;
  availableModelOptions?: string[];
  container?: HTMLElement | null;
  resolveGroupState: (
    groupName: string,
  ) => UpstreamAccountGroupSettingsSnapshot | null;
  saveGroupSettings: (
    groupName: string,
    payload: UpdateUpstreamAccountGroupPayload,
    options: { existing: boolean },
  ) => Promise<unknown>;
  deleteGroupSettings?: (groupName: string) => Promise<void>;
}

type OpenGroupSettingsEditorOptions = {
  onSaved?: (groupName: string) => void;
  onDeleted?: (groupName: string) => void;
};

export function useUpstreamAccountGroupSettingsDialog(
  options: UseUpstreamAccountGroupSettingsDialogOptions,
): {
  openEditor: (
    groupName: string,
    openOptions?: OpenGroupSettingsEditorOptions,
  ) => void;
  closeEditor: () => void;
  dialog: ReactNode;
} {
  const { t, locale } = useTranslation();
  const {
    container,
    availableModelOptions: injectedAvailableModelOptions = [],
    resolveGroupState,
    saveGroupSettings,
    deleteGroupSettings,
    writesEnabled,
  } = options;
  const [editor, setEditor] = useState<GroupSettingsEditorState>(
    createInitialEditorState,
  );
  const [busyAction, setBusyAction] = useState<"save" | "delete" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const fetchedAvailableModelOptions = useAvailableModelOptions(
    writesEnabled && injectedAvailableModelOptions.length === 0,
  );
  const availableModelOptions =
    injectedAvailableModelOptions.length > 0
      ? injectedAvailableModelOptions
      : fetchedAvailableModelOptions;
  const {
    nodes: forwardProxyNodes,
    catalogState: forwardProxyCatalogState,
    refresh: refreshForwardProxyBindings,
  } = useForwardProxyBindingNodes(editor.boundProxyKeys, {
    enabled: editor.open,
    groupName: editor.groupName,
  });
  const currentGroupSnapshot = useMemo(
    () =>
      editor.open && editor.groupName
        ? resolveGroupState(editor.groupName)
        : null,
    [editor.groupName, editor.open, resolveGroupState],
  );

  useGroupNoteCatalogAutoRefresh({
    open: editor.open,
    refresh: refreshForwardProxyBindings,
    catalogState: forwardProxyCatalogState,
  });

  const openEditor = useCallback(
    (groupName: string, openOptions?: OpenGroupSettingsEditorOptions) => {
      if (!writesEnabled) return;
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return;
      const snapshot = resolveGroupState(normalizedGroupName);
      setError(null);
      setEditor({
        open: true,
        groupName: normalizedGroupName,
        note: snapshot?.note ?? "",
        existing: snapshot?.existing === true,
        accountCount: Math.max(0, Math.trunc(snapshot?.accountCount ?? 0)),
        concurrencyLimit: apiConcurrencyLimitToSliderValue(
          snapshot?.concurrencyLimit ?? 0,
        ),
        boundProxyKeys: normalizeBoundProxyKeys(snapshot?.boundProxyKeys),
        nodeShuntEnabled: snapshot?.nodeShuntEnabled === true,
        singleAccountRotationEnabled:
          snapshot?.singleAccountRotationEnabled === true,
        upstream429RetryEnabled: snapshot?.upstream429RetryEnabled === true,
        upstream429MaxRetries: snapshot?.upstream429MaxRetries ?? 0,
        routingRule: snapshot?.routingRule ?? defaultRoutingRule,
        routingRuleDirty: false,
        policyEditorOpen: false,
        onSaved: openOptions?.onSaved ?? null,
        onDeleted: openOptions?.onDeleted ?? null,
      });
    },
    [resolveGroupState, writesEnabled],
  );

  const closeEditor = useCallback(() => {
    if (busyAction != null) return;
    setEditor((current) => ({ ...current, open: false }));
    setError(null);
  }, [busyAction]);

  const handleSave = useCallback(async () => {
    if (!writesEnabled) return;
    const normalizedGroupName = normalizeGroupName(editor.groupName);
    if (!normalizedGroupName) return;

    const normalizedNote = editor.note.trim();
    const normalizedConcurrencyLimit = sliderConcurrencyLimitToApiValue(
      editor.concurrencyLimit,
    );
    const normalizedBoundProxyKeys = normalizeBoundProxyKeys(
      editor.boundProxyKeys,
    );
    const normalizedNodeShuntEnabled = editor.nodeShuntEnabled === true;
    const normalizedSingleAccountRotationEnabled =
      editor.singleAccountRotationEnabled === true;
    const normalizedUpstream429RetryEnabled =
      editor.upstream429RetryEnabled === true;
    const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
      ? normalizeEnabledUpstream429MaxRetries(editor.upstream429MaxRetries)
      : normalizeUpstream429MaxRetries(editor.upstream429MaxRetries);

    setBusyAction("save");
    setError(null);
    try {
      await saveGroupSettings(
        normalizedGroupName,
        {
          note: normalizedNote || undefined,
          boundProxyKeys: normalizedBoundProxyKeys,
          concurrencyLimit: normalizedConcurrencyLimit,
          nodeShuntEnabled: normalizedNodeShuntEnabled,
          singleAccountRotationEnabled: normalizedSingleAccountRotationEnabled,
          upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
          upstream429MaxRetries: normalizedUpstream429MaxRetries,
          ...(editor.routingRuleDirty
            ? { routingRule: editor.routingRule }
            : {}),
        },
        { existing: editor.existing },
      );
      editor.onSaved?.(normalizedGroupName);
      setEditor((current) => ({ ...current, open: false }));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  }, [editor, saveGroupSettings, writesEnabled]);

  const handleDelete = useCallback(async () => {
    if (!writesEnabled || !deleteGroupSettings) return;
    const normalizedGroupName = normalizeGroupName(editor.groupName);
    if (!normalizedGroupName || editor.accountCount > 0) return;

    setBusyAction("delete");
    setError(null);
    try {
      await deleteGroupSettings(normalizedGroupName);
      editor.onDeleted?.(normalizedGroupName);
      setEditor((current) => ({ ...current, open: false }));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusyAction(null);
    }
  }, [deleteGroupSettings, editor, writesEnabled]);

  const dialog = useMemo(
    () => (
      <>
        <UpstreamAccountGroupNoteDialog
          open={editor.open}
          container={container}
          groupName={editor.groupName}
          note={editor.note}
          concurrencyLimit={editor.concurrencyLimit}
          accountCount={editor.accountCount}
          boundProxyKeys={editor.boundProxyKeys}
          nodeShuntEnabled={editor.nodeShuntEnabled}
          singleAccountRotationEnabled={editor.singleAccountRotationEnabled}
          upstream429RetryEnabled={editor.upstream429RetryEnabled}
          upstream429MaxRetries={editor.upstream429MaxRetries}
          onRoutingPolicyEdit={() =>
            setEditor((current) => ({ ...current, policyEditorOpen: true }))
          }
          availableProxyNodes={forwardProxyNodes}
          proxyBindingsCatalogKind={forwardProxyCatalogState.kind}
          proxyBindingsCatalogFreshness={forwardProxyCatalogState.freshness}
          busy={busyAction != null}
          deleting={busyAction === "delete"}
          error={error}
          existing={editor.existing}
          onNoteChange={(value) => {
            setError(null);
            setEditor((current) => ({ ...current, note: value }));
          }}
          onConcurrencyLimitChange={(value) => {
            setError(null);
            setEditor((current) => ({
              ...current,
              concurrencyLimit: value,
            }));
          }}
          onBoundProxyKeysChange={(value) => {
            setError(null);
            setEditor((current) => ({
              ...current,
              boundProxyKeys: value,
            }));
          }}
          onNodeShuntEnabledChange={(value) => {
            setError(null);
            setEditor((current) => ({
              ...current,
              nodeShuntEnabled: value,
            }));
          }}
          onSingleAccountRotationEnabledChange={(value) => {
            setError(null);
            setEditor((current) => ({
              ...current,
              singleAccountRotationEnabled: value,
            }));
          }}
          onUpstream429RetryEnabledChange={(value) => {
            setError(null);
            setEditor((current) => ({
              ...current,
              upstream429RetryEnabled: value,
              upstream429MaxRetries: value
                ? normalizeEnabledUpstream429MaxRetries(
                    current.upstream429MaxRetries,
                  )
                : normalizeUpstream429MaxRetries(current.upstream429MaxRetries),
            }));
          }}
          onUpstream429MaxRetriesChange={(value) => {
            setError(null);
            setEditor((current) => ({
              ...current,
              upstream429MaxRetries: value,
            }));
          }}
          onClose={closeEditor}
          onSave={() => void handleSave()}
          onDelete={
            editor.existing && deleteGroupSettings
              ? () => void handleDelete()
              : undefined
          }
          title={t("accountPool.upstreamAccounts.groupNotes.dialogTitle")}
          existingDescription={t(
            "accountPool.upstreamAccounts.groupNotes.existingDescription",
          )}
          draftDescription={t(
            "accountPool.upstreamAccounts.groupNotes.draftDescription",
          )}
          noteLabel={t("accountPool.upstreamAccounts.fields.note")}
          notePlaceholder={t(
            "accountPool.upstreamAccounts.groupNotes.notePlaceholder",
          )}
          concurrencyLimitLabel={t(
            "accountPool.upstreamAccounts.groupNotes.concurrency.label",
          )}
          concurrencyLimitHint={t(
            "accountPool.upstreamAccounts.groupNotes.concurrency.hint",
          )}
          concurrencyLimitCurrentLabel={t(
            "accountPool.upstreamAccounts.groupNotes.concurrency.current",
          )}
          concurrencyLimitUnlimitedLabel={t(
            "accountPool.upstreamAccounts.groupNotes.concurrency.unlimited",
          )}
          cancelLabel={t("accountPool.upstreamAccounts.actions.cancel")}
          saveLabel={t("accountPool.upstreamAccounts.actions.save")}
          deleteLabel={t("accountPool.upstreamAccounts.actions.delete")}
          deleteDisabledHint={
            editor.accountCount > 0
              ? t(
                  "accountPool.upstreamAccounts.groupNotes.deleteBlockedWithCount",
                  { count: editor.accountCount },
                )
              : undefined
          }
          closeLabel={t("accountPool.upstreamAccounts.actions.cancel")}
          existingBadgeLabel={t(
            "accountPool.upstreamAccounts.groupNotes.badges.existing",
          )}
          draftBadgeLabel={t(
            "accountPool.upstreamAccounts.groupNotes.badges.draft",
          )}
          nodeShuntLabel={t(
            "accountPool.upstreamAccounts.groupNotes.nodeShunt.label",
          )}
          nodeShuntHint={t(
            "accountPool.upstreamAccounts.groupNotes.nodeShunt.hint",
          )}
          nodeShuntToggleLabel={t(
            "accountPool.upstreamAccounts.groupNotes.nodeShunt.toggle",
          )}
          nodeShuntWarning={t(
            "accountPool.upstreamAccounts.groupNotes.nodeShunt.warning",
          )}
          singleAccountRotationLabel={t(
            "accountPool.upstreamAccounts.groupNotes.singleAccountRotation.label",
          )}
          singleAccountRotationHint={t(
            "accountPool.upstreamAccounts.groupNotes.singleAccountRotation.hint",
          )}
          singleAccountRotationToggleLabel={t(
            "accountPool.upstreamAccounts.groupNotes.singleAccountRotation.toggle",
          )}
          routingPolicyLabel={t(
            "accountPool.upstreamAccounts.groupNotes.routingPolicy.label",
          )}
          routingPolicyHint={t(
            "accountPool.upstreamAccounts.groupNotes.routingPolicy.hint",
          )}
          routingPolicyEditLabel={t(
            "accountPool.upstreamAccounts.groupNotes.routingPolicy.edit",
          )}
          proxyBindingsLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.label",
          )}
          proxyBindingsHint={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.hint",
          )}
          proxyBindingsAutomaticLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.automatic",
          )}
          proxyBindingsLoadingLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.loading",
          )}
          proxyBindingsEmptyLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.empty",
          )}
          proxyBindingsMissingLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.missing",
          )}
          proxyBindingsUnavailableLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.unavailable",
          )}
          proxyBindingsChartLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartLabel",
          )}
          proxyBindingsChartSuccessLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartSuccess",
          )}
          proxyBindingsChartFailureLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartFailure",
          )}
          proxyBindingsChartEmptyLabel={t(
            "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartEmpty",
          )}
          proxyBindingsChartTotalLabel={t(
            "live.proxy.table.requestTooltip.total",
          )}
          proxyBindingsChartAriaLabel={t("live.proxy.table.requestTrendAria")}
          proxyBindingsChartInteractionHint={t(
            "live.chart.tooltip.instructions",
          )}
          proxyBindingsChartLocaleTag={locale === "zh" ? "zh-CN" : "en-US"}
        />
        <GroupAccountRoutingRuleDialog
          open={editor.open && editor.policyEditorOpen}
          timeoutOverrideSource="group"
          availableModelOptions={availableModelOptions}
          title={t(
            "accountPool.upstreamAccounts.groupNotes.routingPolicy.title",
          )}
          description={t(
            "accountPool.upstreamAccounts.groupNotes.routingPolicy.description",
          )}
          submitLabel={t(
            "accountPool.upstreamAccounts.groupNotes.routingPolicy.save",
          )}
          rule={editor.routingRule}
          effectiveTimeouts={currentGroupSnapshot?.effectiveTimeouts ?? null}
          timeoutFieldSources={
            currentGroupSnapshot?.timeoutFieldSources ?? null
          }
          busy={busyAction != null}
          onClose={() =>
            setEditor((current) => ({ ...current, policyEditorOpen: false }))
          }
          onSubmit={(payload) => {
            setEditor((current) => ({
              ...current,
              policyEditorOpen: false,
              routingRuleDirty: true,
              routingRule: mergeRoutingRulePatch(current.routingRule, payload),
            }));
          }}
          labels={{
            allowNewConversations: t(
              "accountPool.tags.dialog.allowNewConversations",
            ),
            newConversationHint: t(
              "accountPool.tags.dialog.newConversationHint",
            ),
            allowCutOut: t("accountPool.tags.dialog.allowCutOut"),
            allowCutIn: t("accountPool.tags.dialog.allowCutIn"),
            forbidCutOut: t("accountPool.tags.dialog.forbidCutOut"),
            forbidCutIn: t("accountPool.tags.dialog.forbidCutIn"),
            priorityTier: t("accountPool.tags.dialog.priorityTier"),
            priorityPrimary: t("accountPool.tags.dialog.priorityPrimary"),
            priorityNormal: t("accountPool.tags.dialog.priorityNormal"),
            priorityFallback: t("accountPool.tags.dialog.priorityFallback"),
            fastModeRewriteMode: t(
              "accountPool.tags.dialog.fastModeRewriteMode",
            ),
            fastModeKeepOriginal: t(
              "accountPool.tags.dialog.fastModeKeepOriginal",
            ),
            fastModeFillMissing: t(
              "accountPool.tags.dialog.fastModeFillMissing",
            ),
            fastModeForceAdd: t("accountPool.tags.dialog.fastModeForceAdd"),
            fastModeForceRemove: t(
              "accountPool.tags.dialog.fastModeForceRemove",
            ),
            imageToolRewriteMode: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolRewriteMode",
            ),
            imageToolKeepOriginal: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolKeepOriginal",
            ),
            imageToolFillMissing: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolFillMissing",
            ),
            imageToolForceAdd: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolForceAdd",
            ),
            imageToolForceRemove: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolForceRemove",
            ),
            imageToolRewriteHint: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolRewriteHint",
            ),
            statusChangeReasonSectionTitle: t(
              "accountPool.upstreamAccounts.statusChangeReasons.sectionTitle",
            ),
            statusChangeReasonSectionHint: t(
              "accountPool.upstreamAccounts.statusChangeReasons.sectionHint",
            ),
            statusChangeReasonLabel: (reason: StatusChangeReasonCode) =>
              t(
                `accountPool.upstreamAccounts.statusChangeReasons.reasons.${reason}`,
              ),
            statusChangeReasonToggleEnabled: t(
              "accountPool.upstreamAccounts.statusChangeReasons.toggleEnabled",
            ),
            statusChangeReasonToggleDisabled: t(
              "accountPool.upstreamAccounts.statusChangeReasons.toggleDisabled",
            ),
            upstream429Retry: t(
              "accountPool.upstreamAccounts.groupNotes.upstream429.label",
            ),
            upstream429RetryHint: t(
              "accountPool.upstreamAccounts.groupNotes.upstream429.hint",
            ),
            upstream429RetryToggle: t(
              "accountPool.upstreamAccounts.groupNotes.upstream429.toggle",
            ),
            upstream429RetryCount: t(
              "accountPool.upstreamAccounts.groupNotes.upstream429.countLabel",
            ),
            upstream429RetryCountOnce: t(
              "accountPool.upstreamAccounts.groupNotes.upstream429.countOnce",
            ),
            upstream429RetryCountMany: (count: number) =>
              t(
                "accountPool.upstreamAccounts.groupNotes.upstream429.countMany",
                {
                  count,
                },
              ),
            concurrencyLimit: t("accountPool.tags.dialog.concurrencyLimit"),
            concurrencyHint: t("accountPool.tags.dialog.concurrencyHint"),
            currentValue: t("accountPool.tags.dialog.currentValue"),
            unlimited: t("accountPool.tags.dialog.unlimited"),
            availableModels: t("accountPool.tags.dialog.availableModels"),
            availableModelsHint: t(
              "accountPool.tags.dialog.availableModelsHint",
            ),
            availableModelsSearchPlaceholder: t(
              "accountPool.tags.dialog.availableModelsSearchPlaceholder",
            ),
            availableModelsEmpty: t(
              "accountPool.tags.dialog.availableModelsEmpty",
            ),
            availableModelsAll: t("accountPool.tags.dialog.availableModelsAll"),
            availableModelsCustomLabel: (value) =>
              t("accountPool.tags.dialog.availableModelsCustomLabel", {
                value,
              }),
            availableModelsAddCustom: t(
              "accountPool.tags.dialog.availableModelsAddCustom",
            ),
            availableModelsInherited: t(
              "accountPool.tags.dialog.availableModelsInherited",
            ),
            availableModelsRemove: t(
              "accountPool.tags.dialog.availableModelsRemove",
            ),
            timeoutSectionTitle: t(
              "accountPool.upstreamAccounts.routing.timeout.sectionTitle",
            ),
            timeoutSectionHint: t(
              "accountPool.upstreamAccounts.groupNotes.routingPolicy.description",
            ),
            timeoutResponsesFirstByte: t(
              "accountPool.upstreamAccounts.routing.timeout.responsesFirstByte",
            ),
            timeoutCompactFirstByte: t(
              "accountPool.upstreamAccounts.routing.timeout.compactFirstByte",
            ),
            timeoutResponsesStream: t(
              "accountPool.upstreamAccounts.routing.timeout.responsesStream",
            ),
            timeoutCompactStream: t(
              "accountPool.upstreamAccounts.routing.timeout.compactStream",
            ),
            timeoutInheritedValue: t(
              "accountPool.upstreamAccounts.timeoutEditor.inherited",
            ),
            timeoutOverrideValue: t(
              "accountPool.upstreamAccounts.timeoutEditor.groupOverride",
            ),
            timeoutClearField: t(
              "accountPool.upstreamAccounts.effectiveRule.overrideClear",
            ),
            timeoutInheritField: t(
              "accountPool.tags.dialog.availableModelsInherited",
            ),
            timeoutSourceGlobal: t(
              "accountPool.upstreamAccounts.effectiveRule.sourceRoot",
            ),
            timeoutSourceGroup: t(
              "accountPool.upstreamAccounts.effectiveRule.sourceGroup",
            ),
            timeoutSourceAccount: t(
              "accountPool.upstreamAccounts.effectiveRule.sourceAccount",
            ),
            cancel: t("accountPool.tags.dialog.cancel"),
            validation: t("accountPool.tags.dialog.validation"),
          }}
        />
      </>
    ),
    [
      busyAction,
      availableModelOptions,
      closeEditor,
      container,
      deleteGroupSettings,
      editor.boundProxyKeys,
      editor.accountCount,
      editor.concurrencyLimit,
      editor.existing,
      editor.groupName,
      editor.nodeShuntEnabled,
      editor.note,
      editor.open,
      editor.policyEditorOpen,
      editor.routingRule,
      editor.singleAccountRotationEnabled,
      editor.upstream429MaxRetries,
      editor.upstream429RetryEnabled,
      error,
      currentGroupSnapshot,
      forwardProxyCatalogState.freshness,
      forwardProxyCatalogState.kind,
      forwardProxyNodes,
      handleDelete,
      handleSave,
      locale,
      t,
    ],
  );

  return {
    openEditor,
    closeEditor,
    dialog,
  };
}
