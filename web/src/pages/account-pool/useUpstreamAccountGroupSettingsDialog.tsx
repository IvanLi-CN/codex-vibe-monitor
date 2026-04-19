import { useCallback, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type {
  UpdateUpstreamAccountGroupPayload,
} from "../../lib/api";
import { apiConcurrencyLimitToSliderValue, sliderConcurrencyLimitToApiValue } from "../../lib/concurrencyLimit";
import { normalizeGroupName } from "../../lib/upstreamAccountGroups";
import { useTranslation } from "../../i18n";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { useGroupNoteCatalogAutoRefresh } from "./useGroupNoteCatalogAutoRefresh";

type GroupSettingsEditorState = {
  open: boolean;
  groupName: string;
  note: string;
  existing: boolean;
  concurrencyLimit: number;
  boundProxyKeys: string[];
  nodeShuntEnabled: boolean;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
};

export type UpstreamAccountGroupSettingsSnapshot = {
  groupName: string;
  note?: string | null;
  existing?: boolean;
  concurrencyLimit?: number | null;
  boundProxyKeys?: string[];
  nodeShuntEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
};

function createInitialEditorState(): GroupSettingsEditorState {
  return {
    open: false,
    groupName: "",
    note: "",
    existing: false,
    concurrencyLimit: apiConcurrencyLimitToSliderValue(0),
    boundProxyKeys: [],
    nodeShuntEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
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
}

export function useUpstreamAccountGroupSettingsDialog(
  options: UseUpstreamAccountGroupSettingsDialogOptions,
): {
  openEditor: (groupName: string) => void;
  closeEditor: () => void;
  dialog: ReactNode;
} {
  const { t, locale } = useTranslation();
  const { container, resolveGroupState, saveGroupSettings, writesEnabled } =
    options;
  const [editor, setEditor] = useState<GroupSettingsEditorState>(
    createInitialEditorState,
  );
  const [busy, setBusy] = useState(false);
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
    (groupName: string) => {
      if (!writesEnabled) return;
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return;
      const snapshot = resolveGroupState(normalizedGroupName);
      setError(null);
      setEditor({
        open: true,
        groupName: normalizedGroupName,
        note: snapshot?.note ?? "",
        existing: snapshot?.existing !== false,
        concurrencyLimit: apiConcurrencyLimitToSliderValue(
          snapshot?.concurrencyLimit ?? 0,
        ),
        boundProxyKeys: normalizeBoundProxyKeys(snapshot?.boundProxyKeys),
        nodeShuntEnabled: snapshot?.nodeShuntEnabled === true,
        upstream429RetryEnabled: snapshot?.upstream429RetryEnabled === true,
        upstream429MaxRetries: snapshot?.upstream429MaxRetries ?? 0,
      });
    },
    [resolveGroupState, writesEnabled],
  );

  const closeEditor = useCallback(() => {
    if (busy) return;
    setEditor((current) => ({ ...current, open: false }));
    setError(null);
  }, [busy]);

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
    const normalizedUpstream429RetryEnabled =
      editor.upstream429RetryEnabled === true;
    const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
      ? normalizeEnabledUpstream429MaxRetries(editor.upstream429MaxRetries)
      : normalizeUpstream429MaxRetries(editor.upstream429MaxRetries);

    setBusy(true);
    setError(null);
    try {
      await saveGroupSettings(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
        concurrencyLimit: normalizedConcurrencyLimit,
        nodeShuntEnabled: normalizedNodeShuntEnabled,
        upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
        upstream429MaxRetries: normalizedUpstream429MaxRetries,
      }, { existing: editor.existing });
      setEditor((current) => ({ ...current, open: false }));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }, [editor, saveGroupSettings, writesEnabled]);

  const dialog = useMemo(
    () => (
      <UpstreamAccountGroupNoteDialog
        open={editor.open}
        container={container}
        groupName={editor.groupName}
        note={editor.note}
        concurrencyLimit={editor.concurrencyLimit}
        boundProxyKeys={editor.boundProxyKeys}
        nodeShuntEnabled={editor.nodeShuntEnabled}
        upstream429RetryEnabled={editor.upstream429RetryEnabled}
        upstream429MaxRetries={editor.upstream429MaxRetries}
        availableProxyNodes={forwardProxyNodes}
        proxyBindingsCatalogKind={forwardProxyCatalogState.kind}
        proxyBindingsCatalogFreshness={forwardProxyCatalogState.freshness}
        busy={busy}
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
    ),
    [
      busy,
      closeEditor,
      container,
      editor.boundProxyKeys,
      editor.concurrencyLimit,
      editor.existing,
      editor.groupName,
      editor.nodeShuntEnabled,
      editor.note,
      editor.open,
      editor.upstream429MaxRetries,
      editor.upstream429RetryEnabled,
      error,
      forwardProxyCatalogState.freshness,
      forwardProxyCatalogState.kind,
      forwardProxyNodes,
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
