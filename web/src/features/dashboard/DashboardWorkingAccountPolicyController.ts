import type * as React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  UpdateGroupAccountRoutingRulePayload,
  UpstreamAccountActivityAccount,
} from "../../lib/api";
import { updateUpstreamAccount } from "../../lib/api";
import type { RoutingStateVersion } from "../../lib/api/core-foundation";
import {
  acceptsRoutingStateVersion,
  compareRoutingStateVersion,
} from "../../lib/api/core-foundation";
import { emitUpstreamAccountsChanged } from "../../lib/upstreamAccountsEvents";
import {
  type AccountQuickPolicyDraft,
  accountPolicyDraftFromRoutingRule,
  accountPolicyDraftFromRule,
  accountPolicyDraftsEqual,
  cycleAccountFastModePolicy,
  cycleAccountPriorityPolicy,
} from "./DashboardWorkingAccountPolicy";

type PolicyDraftState = {
  policyDraft: AccountQuickPolicyDraft;
  setPolicyDraft: (draft: AccountQuickPolicyDraft) => void;
  lastCommittedPolicyRef: React.MutableRefObject<AccountQuickPolicyDraft>;
  lastCommittedRoutingStateVersionRef: React.MutableRefObject<RoutingStateVersion | null>;
};

function useAccountPolicyDraftState(
  serverPolicyDraft: AccountQuickPolicyDraft,
  routingStateVersion?: RoutingStateVersion | null,
): PolicyDraftState {
  const [policyDraft, setPolicyDraft] = useState<AccountQuickPolicyDraft>(serverPolicyDraft);
  const lastCommittedPolicyRef = useRef<AccountQuickPolicyDraft>(serverPolicyDraft);
  const lastCommittedRoutingStateVersionRef = useRef<RoutingStateVersion | null>(
    routingStateVersion ?? null,
  );
  return {
    policyDraft,
    setPolicyDraft,
    lastCommittedPolicyRef,
    lastCommittedRoutingStateVersionRef,
  };
}

function useAccountPolicySync({
  serverPolicyDraft,
  routingStateVersion,
  isSavingPolicy,
  debounceTimerRef,
  draftState,
}: {
  serverPolicyDraft: AccountQuickPolicyDraft;
  routingStateVersion?: RoutingStateVersion | null;
  isSavingPolicy: boolean;
  debounceTimerRef: React.MutableRefObject<ReturnType<typeof setTimeout> | null>;
  draftState: PolicyDraftState;
}) {
  const { setPolicyDraft, lastCommittedPolicyRef, lastCommittedRoutingStateVersionRef } =
    draftState;
  useEffect(() => {
    const currentRoutingStateVersion = lastCommittedRoutingStateVersionRef.current;
    if (!acceptsRoutingStateVersion(currentRoutingStateVersion, routingStateVersion, "live")) {
      return;
    }
    if (!routingStateVersion && lastCommittedRoutingStateVersionRef.current) {
      return;
    }
    if (
      currentRoutingStateVersion &&
      routingStateVersion &&
      compareRoutingStateVersion(routingStateVersion, currentRoutingStateVersion) === 0 &&
      !accountPolicyDraftsEqual(serverPolicyDraft, lastCommittedPolicyRef.current)
    ) {
      return;
    }
    lastCommittedPolicyRef.current = serverPolicyDraft;
    lastCommittedRoutingStateVersionRef.current = routingStateVersion ?? null;
    if (!debounceTimerRef.current && !isSavingPolicy) {
      setPolicyDraft(serverPolicyDraft);
    }
  }, [
    isSavingPolicy,
    debounceTimerRef,
    lastCommittedPolicyRef,
    lastCommittedRoutingStateVersionRef,
    routingStateVersion,
    serverPolicyDraft,
    setPolicyDraft,
  ]);
}

type SaveAccountPolicyPatchArgs = {
  accountId: number;
  patch: UpdateGroupAccountRoutingRulePayload;
  updateUi: boolean;
  seq: number;
  saveSeqRef: React.MutableRefObject<number>;
  mountedRef: React.MutableRefObject<boolean>;
  lastCommittedPolicyRef: React.MutableRefObject<AccountQuickPolicyDraft>;
  lastCommittedRoutingStateVersionRef: React.MutableRefObject<RoutingStateVersion | null>;
  setPolicyDraft: (draft: AccountQuickPolicyDraft) => void;
  setPolicySaveError: (message: string | null) => void;
  onPolicyChanged?: () => void;
};

async function saveAccountPolicyPatch({
  accountId,
  patch,
  updateUi,
  seq,
  saveSeqRef,
  mountedRef,
  lastCommittedPolicyRef,
  lastCommittedRoutingStateVersionRef,
  setPolicyDraft,
  setPolicySaveError,
  onPolicyChanged,
}: SaveAccountPolicyPatchArgs) {
  const response = await updateUpstreamAccount(accountId, { routingRule: patch });
  if (saveSeqRef.current !== seq) return;
  const confirmedDraft = accountPolicyDraftFromRoutingRule(response.effectiveRoutingRule);
  const currentVersion = lastCommittedRoutingStateVersionRef.current;
  if (!acceptsRoutingStateVersion(currentVersion, response.routingStateVersion, "patch")) return;
  lastCommittedPolicyRef.current = confirmedDraft;
  // Keep the confirmation fence until a fresh snapshot catches up with a committed write.
  if (response.routingStateVersion) {
    lastCommittedRoutingStateVersionRef.current = response.routingStateVersion;
  }
  if (updateUi && mountedRef.current) {
    setPolicyDraft(confirmedDraft);
    setPolicySaveError(null);
  }
  emitUpstreamAccountsChanged();
  if (mountedRef.current) onPolicyChanged?.();
}

function useAccountPolicyPersistence({
  account,
  onPolicyChanged,
  draftState,
}: {
  account: UpstreamAccountActivityAccount;
  onPolicyChanged?: () => void;
  draftState: PolicyDraftState;
}) {
  const [policySaveError, setPolicySaveError] = useState<string | null>(null);
  const [isSavingPolicy, setIsSavingPolicy] = useState(false);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingPatchRef = useRef<UpdateGroupAccountRoutingRulePayload | null>(null);
  const pendingDraftRef = useRef<AccountQuickPolicyDraft | null>(null);
  const mountedRef = useRef(true);
  const saveSeqRef = useRef(0);
  const flushPolicySaveRef = useRef<((updateUi?: boolean) => Promise<void>) | null>(null);
  const flushPolicySave = useCallback(
    async (updateUi = true) => {
      const accountId = account.upstreamAccountId;
      const patch = pendingPatchRef.current;
      const nextDraft = pendingDraftRef.current;
      pendingPatchRef.current = null;
      pendingDraftRef.current = null;
      debounceTimerRef.current = null;
      if (accountId == null || !patch || !nextDraft) return;
      const seq = saveSeqRef.current + 1;
      saveSeqRef.current = seq;
      const rollbackDraft = draftState.lastCommittedPolicyRef.current;
      if (updateUi && mountedRef.current) setIsSavingPolicy(true);
      try {
        await saveAccountPolicyPatch({
          accountId,
          patch,
          updateUi,
          seq,
          saveSeqRef,
          mountedRef,
          lastCommittedPolicyRef: draftState.lastCommittedPolicyRef,
          lastCommittedRoutingStateVersionRef: draftState.lastCommittedRoutingStateVersionRef,
          setPolicyDraft: draftState.setPolicyDraft,
          setPolicySaveError,
          onPolicyChanged,
        });
      } catch (err) {
        if (saveSeqRef.current !== seq) return;
        if (updateUi && mountedRef.current && !pendingPatchRef.current) {
          draftState.setPolicyDraft(rollbackDraft);
        }
        if (updateUi && mountedRef.current) {
          setPolicySaveError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (updateUi && mountedRef.current && saveSeqRef.current === seq) {
          setIsSavingPolicy(false);
        }
      }
    },
    [account.upstreamAccountId, draftState, onPolicyChanged],
  );
  flushPolicySaveRef.current = flushPolicySave;
  useEffect(
    () => () => {
      mountedRef.current = false;
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
        void flushPolicySaveRef.current?.(false);
      }
    },
    [],
  );
  const schedulePolicySave = useCallback(
    (nextDraft: AccountQuickPolicyDraft, patch: UpdateGroupAccountRoutingRulePayload) => {
      if (account.upstreamAccountId == null) return;
      draftState.setPolicyDraft(nextDraft);
      setPolicySaveError(null);
      pendingPatchRef.current = { ...(pendingPatchRef.current ?? {}), ...patch };
      pendingDraftRef.current = nextDraft;
      if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = setTimeout(() => void flushPolicySave(), 1000);
    },
    [account.upstreamAccountId, draftState, flushPolicySave],
  );
  return {
    policySaveError,
    isSavingPolicy,
    debounceTimerRef,
    schedulePolicySave,
  };
}

function useAccountPolicyActions({
  policyDraft,
  schedulePolicySave,
}: {
  policyDraft: AccountQuickPolicyDraft;
  schedulePolicySave: (
    nextDraft: AccountQuickPolicyDraft,
    patch: UpdateGroupAccountRoutingRulePayload,
  ) => void;
}) {
  const handleCyclePriorityPolicy = useCallback(() => {
    const nextDraft = cycleAccountPriorityPolicy(policyDraft);
    schedulePolicySave(nextDraft, { priorityTier: nextDraft.priorityTier });
  }, [policyDraft, schedulePolicySave]);
  const handleToggleCutOut = useCallback(() => {
    const nextDraft = { ...policyDraft, allowCutOut: !policyDraft.allowCutOut };
    schedulePolicySave(nextDraft, { allowCutOut: nextDraft.allowCutOut });
  }, [policyDraft, schedulePolicySave]);
  const handleToggleCutIn = useCallback(() => {
    const nextDraft = { ...policyDraft, allowCutIn: !policyDraft.allowCutIn };
    schedulePolicySave(nextDraft, { allowCutIn: nextDraft.allowCutIn });
  }, [policyDraft, schedulePolicySave]);
  const handleCycleFastModePolicy = useCallback(() => {
    const nextDraft = cycleAccountFastModePolicy(policyDraft);
    schedulePolicySave(nextDraft, { fastModeRewriteMode: nextDraft.fastModeRewriteMode });
  }, [policyDraft, schedulePolicySave]);
  return {
    handleCyclePriorityPolicy,
    handleToggleCutOut,
    handleToggleCutIn,
    handleCycleFastModePolicy,
  };
}

export function useDashboardWorkingAccountPolicyController({
  account,
  routingStateVersion,
  onPolicyChanged,
}: {
  account: UpstreamAccountActivityAccount;
  routingStateVersion?: RoutingStateVersion | null;
  onPolicyChanged?: () => void;
}) {
  const serverPolicyDraft = useMemo(() => accountPolicyDraftFromRule(account), [account]);
  const draftState = useAccountPolicyDraftState(serverPolicyDraft, routingStateVersion);
  const persistence = useAccountPolicyPersistence({ account, onPolicyChanged, draftState });
  useAccountPolicySync({
    serverPolicyDraft,
    routingStateVersion,
    isSavingPolicy: persistence.isSavingPolicy,
    debounceTimerRef: persistence.debounceTimerRef,
    draftState,
  });
  const actions = useAccountPolicyActions({
    policyDraft: draftState.policyDraft,
    schedulePolicySave: persistence.schedulePolicySave,
  });
  return {
    policyDraft: draftState.policyDraft,
    policySaveError: persistence.policySaveError,
    isSavingPolicy: persistence.isSavingPolicy,
    isPolicySaveScheduled: persistence.debounceTimerRef.current != null,
    ...actions,
  };
}

export type DashboardWorkingAccountPolicyController = ReturnType<
  typeof useDashboardWorkingAccountPolicyController
>;
