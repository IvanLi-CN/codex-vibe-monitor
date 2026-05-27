import { useCallback, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type {
  TagRoutingRule,
  UpdateUpstreamAccountGroupPayload,
} from "../../lib/api";
import { apiConcurrencyLimitToSliderValue, sliderConcurrencyLimitToApiValue } from "../../lib/concurrencyLimit";
import { normalizeGroupName } from "../../lib/upstreamAccountGroups";
import { useTranslation } from "../../i18n";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { TagRuleDialog } from "../../components/TagRuleDialog";
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
  routingRule: TagRoutingRule;
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
  routingRule?: TagRoutingRule;
};

const defaultRoutingRule: TagRoutingRule = {
  blockNewConversations: false,
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: "normal",
  fastModeRewriteMode: "keep_original",
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
};

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
}

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
    resolveGroupState,
    saveGroupSettings,
    deleteGroupSettings,
    writesEnabled,
  } =
    options;
  const [editor, setEditor] = useState<GroupSettingsEditorState>(
    createInitialEditorState,
  );
  const [busyAction, setBusyAction] = useState<"save" | "delete" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const {
    nodes: forwardProxyNodes,
    catalogState: forwardProxyCatalogState,
    refresh: refreshForwardProxyBindings,
  } = useForwardProxyBindingNodes(editor.boundProxyKeys, {
    enabled: editor.open,
    groupName: editor.groupName,
  });

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
      await saveGroupSettings(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
        concurrencyLimit: normalizedConcurrencyLimit,
        nodeShuntEnabled: normalizedNodeShuntEnabled,
        singleAccountRotationEnabled: normalizedSingleAccountRotationEnabled,
        upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
        upstream429MaxRetries: normalizedUpstream429MaxRetries,
        ...(editor.routingRuleDirty ? { routingRule: editor.routingRule } : {}),
      }, { existing: editor.existing });
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
        upstream429RetryLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.label",
        )}
        upstream429RetryHint={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.hint",
        )}
        upstream429RetryToggleLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.toggle",
        )}
        upstream429RetryCountLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.countLabel",
        )}
        upstream429RetryCountOptions={[1, 2, 3, 4, 5].map((value) => ({
          value,
          label:
            value === 1
              ? t(
                  "accountPool.upstreamAccounts.groupNotes.upstream429.countOnce",
                )
              : t(
                  "accountPool.upstreamAccounts.groupNotes.upstream429.countMany",
                  { count: value },
                ),
        }))}
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
        proxyBindingsChartTotalLabel={t("live.proxy.table.requestTooltip.total")}
        proxyBindingsChartAriaLabel={t("live.proxy.table.requestTrendAria")}
        proxyBindingsChartInteractionHint={t("live.chart.tooltip.instructions")}
        proxyBindingsChartLocaleTag={locale === "zh" ? "zh-CN" : "en-US"}
      />
      <TagRuleDialog
        open={editor.open && editor.policyEditorOpen}
        mode="edit"
        policyOnly
        title={t("accountPool.upstreamAccounts.groupNotes.routingPolicy.title")}
        description={t(
          "accountPool.upstreamAccounts.groupNotes.routingPolicy.description",
        )}
        submitLabel={t(
          "accountPool.upstreamAccounts.groupNotes.routingPolicy.save",
        )}
        tag={{
          id: 0,
          name: editor.groupName,
          routingRule: editor.routingRule,
          accountCount: editor.accountCount,
          groupCount: 1,
          updatedAt: "",
        }}
        busy={busyAction != null}
        onClose={() =>
          setEditor((current) => ({ ...current, policyEditorOpen: false }))
        }
        onSubmit={(payload) => {
          setEditor((current) => ({
            ...current,
            policyEditorOpen: false,
            routingRuleDirty: true,
            routingRule: {
              blockNewConversations: payload.blockNewConversations ?? false,
              allowCutOut: payload.allowCutOut ?? true,
              allowCutIn: payload.allowCutIn ?? true,
              priorityTier: payload.priorityTier ?? "normal",
              fastModeRewriteMode:
                payload.fastModeRewriteMode ?? "keep_original",
              concurrencyLimit: payload.concurrencyLimit ?? 0,
              upstream429RetryEnabled:
                payload.upstream429RetryEnabled === true,
              upstream429MaxRetries: payload.upstream429MaxRetries ?? 0,
            },
          }));
        }}
        labels={{
          createTitle: t("accountPool.tags.dialog.createTitle"),
          editTitle: t("accountPool.tags.dialog.editTitle"),
          description: t("accountPool.tags.dialog.description"),
          name: t("accountPool.tags.dialog.name"),
          namePlaceholder: t("accountPool.tags.dialog.namePlaceholder"),
          blockNewConversations: t("accountPool.tags.dialog.blockNewConversations"),
          forbidNewConversation: t("accountPool.tags.dialog.forbidNewConversation"),
          allowCutOut: t("accountPool.tags.dialog.allowCutOut"),
          allowCutIn: t("accountPool.tags.dialog.allowCutIn"),
          forbidCutOut: t("accountPool.tags.dialog.forbidCutOut"),
          forbidCutIn: t("accountPool.tags.dialog.forbidCutIn"),
          priorityTier: t("accountPool.tags.dialog.priorityTier"),
          priorityPrimary: t("accountPool.tags.dialog.priorityPrimary"),
          priorityNormal: t("accountPool.tags.dialog.priorityNormal"),
          priorityFallback: t("accountPool.tags.dialog.priorityFallback"),
          fastModeRewriteMode: t("accountPool.tags.dialog.fastModeRewriteMode"),
          fastModeKeepOriginal: t("accountPool.tags.dialog.fastModeKeepOriginal"),
          fastModeFillMissing: t("accountPool.tags.dialog.fastModeFillMissing"),
          fastModeForceAdd: t("accountPool.tags.dialog.fastModeForceAdd"),
          fastModeForceRemove: t("accountPool.tags.dialog.fastModeForceRemove"),
          concurrencyLimit: t("accountPool.tags.dialog.concurrencyLimit"),
          concurrencyHint: t("accountPool.tags.dialog.concurrencyHint"),
          currentValue: t("accountPool.tags.dialog.currentValue"),
          unlimited: t("accountPool.tags.dialog.unlimited"),
          cancel: t("accountPool.tags.dialog.cancel"),
          save: t("accountPool.tags.dialog.save"),
          create: t("accountPool.tags.dialog.createAction"),
          validation: t("accountPool.tags.dialog.validation"),
        }}
      />
      </>
    ),
    [
      busyAction,
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
