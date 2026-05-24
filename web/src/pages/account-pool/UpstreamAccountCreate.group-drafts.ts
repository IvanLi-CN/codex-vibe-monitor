/* eslint-disable react-hooks/exhaustive-deps */
import { useCallback, useMemo } from "react";
import {
  isExistingGroup,
  normalizeGroupName,
  resolveGroupConcurrencyLimit,
  resolveGroupNote,
} from "../../lib/upstreamAccountGroups";
import {
  apiConcurrencyLimitToSliderValue,
  sliderConcurrencyLimitToApiValue,
} from "../../lib/concurrencyLimit";
import {
  resolvePersistedGroupNodeShuntEnabled,
  resolvePersistedGroupSingleAccountRotationEnabled,
} from "../../lib/upstreamAccountGroupDrafts";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import {
  type BatchOauthRow,
  formatImportedOauthSelectionLabel,
  type GroupNoteEditorState,
  normalizeBoundProxyKeys,
  normalizeEnabledGroupUpstream429MaxRetries,
  normalizeGroupUpstream429MaxRetries,
} from "./UpstreamAccountCreate.shared";

type OpenGroupNoteEditorOptions = {
  onSaved?: (groupName: string) => void;
  onDeleted?: (groupName: string) => void;
};

type GroupSummaryLike = {
  groupName: string;
  boundProxyKeys?: string[];
  nodeShuntEnabled?: boolean;
  singleAccountRotationEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number | null;
};

type ForwardProxyNodeLike = {
  key: string;
  selectable?: boolean;
  aliasKeys?: string[] | null;
};

export function useUpstreamAccountCreateGroupDrafts(
  ctx: UpstreamAccountCreateControllerContext,
) {
  const {
    apiKeyGroupName,
    batchDefaultGroupName,
    batchRows,
    forwardProxyNodes,
    forwardProxyCatalogState,
    groupDraftBoundProxyKeys,
    groupDraftConcurrencyLimits,
    groupDraftNodeShuntEnabled,
    groupDraftSingleAccountRotationEnabled,
    groupDraftNotes,
    groupDraftUpstream429MaxRetries,
    groupDraftUpstream429RetryEnabled,
    groupNoteBusy,
    groupNoteEditor,
    groups,
    importFiles,
    importGroupName,
    invalidateSingleOauthSessionForMetadataEdit,
    locale,
    oauthGroupName,
    persistedGroupNoteSyncDrafts,
    deleteGroupNote,
    saveGroupNote,
    setGroupDraftBoundProxyKeys,
    setGroupDraftConcurrencyLimits,
    setGroupDraftNodeShuntEnabled,
    setGroupDraftSingleAccountRotationEnabled,
    setGroupDraftNotes,
    setGroupDraftUpstream429MaxRetries,
    setGroupDraftUpstream429RetryEnabled,
    setGroupNoteBusy,
    setGroupNoteEditor,
    setGroupNoteError,
    setPersistedGroupNoteSyncDrafts,
    setApiKeyGroupName,
    setBatchDefaultGroupName,
    setBatchRows,
    t,
    setImportGroupName,
    setOauthGroupName,
    writesEnabled,
  } = ctx;

  function resolveGroupSummaryForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return null;
    return (
      groups.find(
        (group: GroupSummaryLike) => normalizeGroupName(group.groupName) === normalized,
      ) ?? null
    );
  }
  function resolveGroupNoteForName(groupName: string) {
    return resolveGroupNote(groups, groupDraftNotes, groupName);
  }
  function resolveGroupConcurrencyLimitForName(groupName: string) {
    return resolveGroupConcurrencyLimit(
      groups,
      groupDraftConcurrencyLimits,
      groupName,
    );
  }
  function resolveGroupBoundProxyKeysForName(groupName: string) {
    return (
      resolveGroupSummaryForName(groupName)?.boundProxyKeys ??
      groupDraftBoundProxyKeys[normalizeGroupName(groupName)] ??
      []
    );
  }
  function resolveGroupNodeShuntEnabledForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return false;
    const existingGroup = resolveGroupSummaryForName(normalized);
    if (existingGroup) {
      return existingGroup.nodeShuntEnabled === true;
    }
    return groupDraftNodeShuntEnabled[normalized] === true;
  }
  function resolveGroupSingleAccountRotationEnabledForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return false;
    const existingGroup = resolveGroupSummaryForName(normalized);
    if (existingGroup) {
      return existingGroup.singleAccountRotationEnabled === true;
    }
    return groupDraftSingleAccountRotationEnabled[normalized] === true;
  }
  function resolveGroupUpstream429RetryEnabledForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return false;
    const existingGroup = resolveGroupSummaryForName(normalized);
    if (existingGroup) {
      return existingGroup.upstream429RetryEnabled === true;
    }
    return groupDraftUpstream429RetryEnabled[normalized] === true;
  }
  function resolveGroupUpstream429MaxRetriesForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return 0;
    const existingGroup = resolveGroupSummaryForName(normalized);
    const retryEnabled = existingGroup
      ? existingGroup.upstream429RetryEnabled === true
      : groupDraftUpstream429RetryEnabled[normalized] === true;
    const rawValue = existingGroup
      ? existingGroup.upstream429MaxRetries
      : groupDraftUpstream429MaxRetries[normalized];
    return retryEnabled
      ? normalizeEnabledGroupUpstream429MaxRetries(rawValue)
      : normalizeGroupUpstream429MaxRetries(rawValue);
  }
  function resolvePendingGroupNoteForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return "";
    if (isExistingGroup(groups, normalized)) {
      return persistedGroupNoteSyncDrafts[normalized]?.trim() ?? "";
    }
    return groupDraftNotes[normalized]?.trim() ?? "";
  }
  function shouldIncludePendingGroupNoteForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return false;
    if (isExistingGroup(groups, normalized)) {
      return normalized in persistedGroupNoteSyncDrafts;
    }
    return normalized in groupDraftNotes;
  }
  function resolvePendingGroupConcurrencyLimitForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return 0;
    if (isExistingGroup(groups, normalized)) {
      return resolveGroupConcurrencyLimitForName(normalized);
    }
    return groupDraftConcurrencyLimits[normalized] ?? 0;
  }
  function resolvePendingGroupBoundProxyKeysForName(groupName: string) {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return [];
    if (isExistingGroup(groups, normalized)) {
      return resolveGroupBoundProxyKeysForName(normalized);
    }
    return normalizeBoundProxyKeys(groupDraftBoundProxyKeys[normalized]);
  }
  function hasGroupSettings(groupName: string) {
    return (
      resolveGroupNoteForName(groupName).trim().length > 0 ||
      resolvePendingGroupBoundProxyKeysForName(groupName).length > 0 ||
      resolveGroupConcurrencyLimitForName(groupName) > 0 ||
      resolveGroupNodeShuntEnabledForName(groupName) ||
      resolveGroupSingleAccountRotationEnabledForName(groupName) ||
      resolveGroupUpstream429RetryEnabledForName(groupName) ||
      resolveGroupUpstream429MaxRetriesForName(groupName) > 0
    );
  }

  const hasLoadedForwardProxyCatalog =
    forwardProxyCatalogState?.kind === "ready-empty" ||
    forwardProxyCatalogState?.kind === "ready-with-data";
  const selectableForwardProxyKeys = useMemo(
    () =>
      new Set(
        (forwardProxyNodes ?? [])
          .filter((node: ForwardProxyNodeLike) => node.selectable)
          .flatMap((node: ForwardProxyNodeLike) => [node.key, ...(node.aliasKeys ?? [])])
          .map((value: string) => value.trim())
          .filter((value: string) => value.length > 0),
      ),
    [forwardProxyNodes],
  );

  const resolveRequiredGroupProxyState = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      const isChinese = locale.toLocaleLowerCase().startsWith("zh");
      if (!normalizedGroupName) {
        return {
          normalizedGroupName: "",
          boundProxyKeys: [] as string[],
          nodeShuntEnabled: false,
          error: isChinese
            ? "必须先选择一个分组。"
            : "Select a group before continuing.",
        };
      }
      const boundProxyKeys =
        resolvePendingGroupBoundProxyKeysForName(normalizedGroupName);
      const nodeShuntEnabled =
        resolveGroupNodeShuntEnabledForName(normalizedGroupName);
      if (boundProxyKeys.length === 0) {
        return {
          normalizedGroupName,
          boundProxyKeys,
          nodeShuntEnabled,
          error: isChinese
            ? `分组“${normalizedGroupName}”还没有绑定代理节点。`
            : `Group "${normalizedGroupName}" does not have any bound proxy nodes.`,
        };
      }
      if (
        hasLoadedForwardProxyCatalog &&
        !nodeShuntEnabled &&
        !boundProxyKeys.some((proxyKey: string) =>
          selectableForwardProxyKeys.has(proxyKey),
        )
      ) {
        return {
          normalizedGroupName,
          boundProxyKeys,
          nodeShuntEnabled,
          error: isChinese
            ? `分组“${normalizedGroupName}”绑定的代理节点当前都不可用。`
            : `Group "${normalizedGroupName}" does not have any selectable bound proxy nodes.`,
        };
      }
      return {
        normalizedGroupName,
        boundProxyKeys,
        nodeShuntEnabled,
        error: null,
      };
    },
    [
      hasLoadedForwardProxyCatalog,
      locale,
      resolveGroupNodeShuntEnabledForName,
      resolvePendingGroupBoundProxyKeysForName,
      selectableForwardProxyKeys,
    ],
  );

  const oauthGroupProxyState = useMemo(
    () => resolveRequiredGroupProxyState(oauthGroupName),
    [oauthGroupName, resolveRequiredGroupProxyState],
  );
  const importGroupProxyState = useMemo(
    () => resolveRequiredGroupProxyState(importGroupName),
    [importGroupName, resolveRequiredGroupProxyState],
  );
  const importSelectionLabel = useMemo(
    () => formatImportedOauthSelectionLabel(importFiles, t),
    [importFiles, t],
  );
  const apiKeyGroupProxyState = useMemo(
    () => resolveRequiredGroupProxyState(apiKeyGroupName),
    [apiKeyGroupName, resolveRequiredGroupProxyState],
  );

  const clearDraftGroupSettings = useCallback((groupName: string) => {
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return;
    setGroupDraftNotes((current: Record<string, string>) => {
      if (!(normalized in current)) return current;
      const next = { ...current };
      delete next[normalized];
      return next;
    });
    setGroupDraftBoundProxyKeys((current: Record<string, string[]>) => {
      if (!(normalized in current)) return current;
      const next = { ...current };
      delete next[normalized];
      return next;
    });
    setGroupDraftNodeShuntEnabled((current: Record<string, boolean>) => {
      if (!(normalized in current)) return current;
      const next = { ...current };
      delete next[normalized];
      return next;
    });
    setGroupDraftSingleAccountRotationEnabled(
      (current: Record<string, boolean>) => {
        if (!(normalized in current)) return current;
        const next = { ...current };
        delete next[normalized];
        return next;
      },
    );
    setGroupDraftConcurrencyLimits((current: Record<string, number>) => {
      if (!(normalized in current)) return current;
      const next = { ...current };
      delete next[normalized];
      return next;
    });
    setGroupDraftUpstream429RetryEnabled((current: Record<string, boolean>) => {
      if (!(normalized in current)) return current;
      const next = { ...current };
      delete next[normalized];
      return next;
    });
    setGroupDraftUpstream429MaxRetries((current: Record<string, number>) => {
      if (!(normalized in current)) return current;
      const next = { ...current };
      delete next[normalized];
      return next;
    });
  }, []);

  const clearSelectedGroupReferences = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return;
      if (normalizeGroupName(oauthGroupName) === normalizedGroupName) {
        setOauthGroupName("");
      }
      if (normalizeGroupName(importGroupName) === normalizedGroupName) {
        setImportGroupName("");
      }
      if (normalizeGroupName(apiKeyGroupName) === normalizedGroupName) {
        setApiKeyGroupName("");
      }
      const deletedDefaultGroup =
        normalizeGroupName(batchDefaultGroupName) === normalizedGroupName;
      if (deletedDefaultGroup) {
        setBatchDefaultGroupName("");
      }
      if (
        Array.isArray(batchRows) &&
        batchRows.some(
          (row: BatchOauthRow) =>
            normalizeGroupName(row.groupName) === normalizedGroupName,
        )
      ) {
        setBatchRows((current: BatchOauthRow[]) =>
          current.map((row: BatchOauthRow) => {
            if (normalizeGroupName(row.groupName) !== normalizedGroupName) {
              return row;
            }
            return {
              ...row,
              groupName: "",
              inheritsDefaultGroup:
                deletedDefaultGroup && row.inheritsDefaultGroup === true,
              metadataError: null,
              actionError: null,
            };
          }),
        );
      }
    },
    [
      apiKeyGroupName,
      batchDefaultGroupName,
      batchRows,
      importGroupName,
      oauthGroupName,
      setApiKeyGroupName,
      setBatchDefaultGroupName,
      setBatchRows,
      setImportGroupName,
      setOauthGroupName,
    ],
  );

  const persistDraftGroupSettings = useCallback(
    async (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return;
      const hasDraftNote = normalizedGroupName in groupDraftNotes;
      const hasDraftBoundProxyKeys =
        normalizedGroupName in groupDraftBoundProxyKeys;
      const hasDraftConcurrencyLimit =
        normalizedGroupName in groupDraftConcurrencyLimits;
      const hasDraftNodeShuntEnabled =
        normalizedGroupName in groupDraftNodeShuntEnabled;
      const hasDraftSingleAccountRotationEnabled =
        normalizedGroupName in groupDraftSingleAccountRotationEnabled;
      const hasDraftUpstream429RetryEnabled =
        normalizedGroupName in groupDraftUpstream429RetryEnabled;
      const hasDraftUpstream429MaxRetries =
        normalizedGroupName in groupDraftUpstream429MaxRetries;
      if (
        !hasDraftNote &&
        !hasDraftBoundProxyKeys &&
        !hasDraftConcurrencyLimit &&
        !hasDraftNodeShuntEnabled &&
        !hasDraftSingleAccountRotationEnabled &&
        !hasDraftUpstream429RetryEnabled &&
        !hasDraftUpstream429MaxRetries
      ) {
        return;
      }
      const normalizedNote = hasDraftNote
        ? (groupDraftNotes[normalizedGroupName]?.trim() ?? "")
        : "";
      const normalizedBoundProxyKeys = hasDraftBoundProxyKeys
        ? normalizeBoundProxyKeys(groupDraftBoundProxyKeys[normalizedGroupName])
        : [];
      const normalizedConcurrencyLimit = hasDraftConcurrencyLimit
        ? (groupDraftConcurrencyLimits[normalizedGroupName] ?? 0)
        : 0;
      const normalizedNodeShuntEnabled = resolvePersistedGroupNodeShuntEnabled(
        hasDraftNodeShuntEnabled,
        groupDraftNodeShuntEnabled[normalizedGroupName],
        resolveGroupNodeShuntEnabledForName(normalizedGroupName),
      );
      const normalizedSingleAccountRotationEnabled =
        resolvePersistedGroupSingleAccountRotationEnabled(
          hasDraftSingleAccountRotationEnabled,
          groupDraftSingleAccountRotationEnabled[normalizedGroupName],
          resolveGroupSingleAccountRotationEnabledForName(normalizedGroupName),
        );
      const normalizedUpstream429RetryEnabled = hasDraftUpstream429RetryEnabled
        ? groupDraftUpstream429RetryEnabled[normalizedGroupName] === true
        : false;
      const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
        ? normalizeEnabledGroupUpstream429MaxRetries(
            groupDraftUpstream429MaxRetries[normalizedGroupName],
          )
        : normalizeGroupUpstream429MaxRetries(
            groupDraftUpstream429MaxRetries[normalizedGroupName],
          );
      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
        concurrencyLimit: normalizedConcurrencyLimit,
        nodeShuntEnabled: normalizedNodeShuntEnabled,
        singleAccountRotationEnabled: normalizedSingleAccountRotationEnabled,
        upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
        upstream429MaxRetries: normalizedUpstream429MaxRetries,
      });
      clearDraftGroupSettings(normalizedGroupName);
    },
    [
      clearDraftGroupSettings,
      groupDraftBoundProxyKeys,
      groupDraftConcurrencyLimits,
      groupDraftNodeShuntEnabled,
      groupDraftSingleAccountRotationEnabled,
      groupDraftNotes,
      groupDraftUpstream429MaxRetries,
      groupDraftUpstream429RetryEnabled,
      resolveGroupNodeShuntEnabledForName,
      resolveGroupSingleAccountRotationEnabledForName,
      saveGroupNote,
    ],
  );

  const openGroupNoteEditor = (
    groupName: string,
    options?: OpenGroupNoteEditorOptions,
  ) => {
    if (!writesEnabled) return;
    const normalized = normalizeGroupName(groupName);
    if (!normalized) return;
    const existingGroup = resolveGroupSummaryForName(normalized);
    setGroupNoteError(null);
    setGroupNoteEditor({
      open: true,
      groupName: normalized,
      note: existingGroup?.note ?? resolveGroupNoteForName(normalized),
      existing: existingGroup != null,
      accountCount: existingGroup?.accountCount ?? 0,
      concurrencyLimit: apiConcurrencyLimitToSliderValue(
        resolveGroupConcurrencyLimitForName(normalized),
      ),
      boundProxyKeys:
        existingGroup?.boundProxyKeys ??
        resolvePendingGroupBoundProxyKeysForName(normalized),
      nodeShuntEnabled: resolveGroupNodeShuntEnabledForName(normalized),
      singleAccountRotationEnabled:
        resolveGroupSingleAccountRotationEnabledForName(normalized),
      upstream429RetryEnabled:
        existingGroup?.upstream429RetryEnabled ??
        resolveGroupUpstream429RetryEnabledForName(normalized),
      upstream429MaxRetries:
        existingGroup?.upstream429RetryEnabled === true
          ? normalizeEnabledGroupUpstream429MaxRetries(
              existingGroup.upstream429MaxRetries,
            )
          : resolveGroupUpstream429MaxRetriesForName(normalized),
      onSaved: options?.onSaved ?? null,
      onDeleted: options?.onDeleted ?? null,
    });
  };

  const closeGroupNoteEditor = () => {
    if (groupNoteBusy) return;
    setGroupNoteEditor((current: GroupNoteEditorState) => ({ ...current, open: false }));
    setGroupNoteError(null);
  };

  const handleSaveGroupNote = async () => {
    if (!writesEnabled) return;
    const normalizedGroupName = normalizeGroupName(groupNoteEditor.groupName);
    if (!normalizedGroupName) return;
    const normalizedNote = groupNoteEditor.note.trim();
    const normalizedConcurrencyLimit = sliderConcurrencyLimitToApiValue(
      groupNoteEditor.concurrencyLimit,
    );
    const normalizedBoundProxyKeys = normalizeBoundProxyKeys(
      groupNoteEditor.boundProxyKeys,
    );
    const normalizedNodeShuntEnabled =
      groupNoteEditor.nodeShuntEnabled === true;
    const normalizedSingleAccountRotationEnabled =
      groupNoteEditor.singleAccountRotationEnabled === true;
    const normalizedUpstream429RetryEnabled =
      groupNoteEditor.upstream429RetryEnabled === true;
    const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
      ? normalizeEnabledGroupUpstream429MaxRetries(
          groupNoteEditor.upstream429MaxRetries,
        )
      : normalizeGroupUpstream429MaxRetries(
          groupNoteEditor.upstream429MaxRetries,
        );
    const currentOauthGroupName = normalizeGroupName(oauthGroupName);
    const currentOauthGroupNote =
      resolvePendingGroupNoteForName(oauthGroupName).trim();
    const currentOauthGroupConcurrencyLimit =
      resolvePendingGroupConcurrencyLimitForName(oauthGroupName);
    const currentOauthGroupBoundProxyKeys =
      resolvePendingGroupBoundProxyKeysForName(oauthGroupName);
    const currentOauthGroupNodeShuntEnabled =
      resolveGroupNodeShuntEnabledForName(oauthGroupName);
    const shouldInvalidateSingleOauthSessionForGroupMetadataChange =
      currentOauthGroupName === normalizedGroupName &&
      (currentOauthGroupNote !== normalizedNote ||
        currentOauthGroupConcurrencyLimit !== normalizedConcurrencyLimit ||
        currentOauthGroupNodeShuntEnabled !== normalizedNodeShuntEnabled ||
        resolveGroupSingleAccountRotationEnabledForName(oauthGroupName) !==
          normalizedSingleAccountRotationEnabled ||
        currentOauthGroupBoundProxyKeys.length !==
          normalizedBoundProxyKeys.length ||
        currentOauthGroupBoundProxyKeys.some(
          (value: string, index: number) => value !== normalizedBoundProxyKeys[index],
        ));
    setGroupNoteError(null);
    setGroupNoteBusy(true);
    try {
      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
        concurrencyLimit: normalizedConcurrencyLimit,
        nodeShuntEnabled: normalizedNodeShuntEnabled,
        singleAccountRotationEnabled: normalizedSingleAccountRotationEnabled,
        upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
        upstream429MaxRetries: normalizedUpstream429MaxRetries,
      });
      if (groupNoteEditor.existing) {
        setPersistedGroupNoteSyncDrafts((current: Record<string, string>) => ({
          ...current,
          [normalizedGroupName]: normalizedNote,
        }));
      }
      groupNoteEditor.onSaved?.(normalizedGroupName);
      if (shouldInvalidateSingleOauthSessionForGroupMetadataChange) {
        invalidateSingleOauthSessionForMetadataEdit();
      }
      clearDraftGroupSettings(normalizedGroupName);
      setGroupNoteEditor((current: GroupNoteEditorState) => ({ ...current, open: false }));
    } catch (err) {
      setGroupNoteError(err instanceof Error ? err.message : String(err));
    } finally {
      setGroupNoteBusy(false);
    }
  };

  const handleDeleteGroupNote = async () => {
    if (!writesEnabled) return;
    const normalizedGroupName = normalizeGroupName(groupNoteEditor.groupName);
    if (!normalizedGroupName || groupNoteEditor.accountCount > 0) return;
    setGroupNoteError(null);
    setGroupNoteBusy(true);
    try {
      await deleteGroupNote(normalizedGroupName);
      clearDraftGroupSettings(normalizedGroupName);
      setPersistedGroupNoteSyncDrafts((current: Record<string, string>) => {
        if (!(normalizedGroupName in current)) {
          return current;
        }
        const next = { ...current };
        delete next[normalizedGroupName];
        return next;
      });
      clearSelectedGroupReferences(normalizedGroupName);
      groupNoteEditor.onDeleted?.(normalizedGroupName);
      setGroupNoteEditor((current: GroupNoteEditorState) => ({
        ...current,
        open: false,
      }));
      if (normalizeGroupName(oauthGroupName) === normalizedGroupName) {
        invalidateSingleOauthSessionForMetadataEdit();
      }
    } catch (err) {
      setGroupNoteError(err instanceof Error ? err.message : String(err));
    } finally {
      setGroupNoteBusy(false);
    }
  };


  return {
    resolveGroupSummaryForName,
    resolveGroupNoteForName,
    resolveGroupConcurrencyLimitForName,
    resolveGroupBoundProxyKeysForName,
    resolveGroupNodeShuntEnabledForName,
    resolveGroupSingleAccountRotationEnabledForName,
    resolveGroupUpstream429RetryEnabledForName,
    resolveGroupUpstream429MaxRetriesForName,
    resolvePendingGroupNoteForName,
    shouldIncludePendingGroupNoteForName,
    resolvePendingGroupConcurrencyLimitForName,
    resolvePendingGroupBoundProxyKeysForName,
    hasGroupSettings,
    resolveRequiredGroupProxyState,
    oauthGroupProxyState,
    importGroupProxyState,
    importSelectionLabel,
    apiKeyGroupProxyState,
    clearDraftGroupSettings,
    persistDraftGroupSettings,
    openGroupNoteEditor,
    closeGroupNoteEditor,
    handleSaveGroupNote,
    handleDeleteGroupNote,
  };
}
