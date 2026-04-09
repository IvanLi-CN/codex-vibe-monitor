export type AccountDraft = {
  displayName: string;
  groupName: string;
  isMother: boolean;
  note: string;
  upstreamBaseUrl: string;
  tagIds: number[];
  localPrimaryLimit: string;
  localSecondaryLimit: string;
  localLimitUnit: string;
  apiKey: string;
};

function areAccountDraftTagIdsEqual(left: number[], right: number[]): boolean {
  if (left.length !== right.length) return false;
  const leftSorted = [...left].sort((a, b) => a - b);
  const rightSorted = [...right].sort((a, b) => a - b);
  return leftSorted.every((tagId, index) => tagId === rightSorted[index]);
}

export function areAccountDraftsEqual(
  left: AccountDraft,
  right: AccountDraft,
): boolean {
  return (
    left.displayName === right.displayName &&
    left.groupName === right.groupName &&
    left.isMother === right.isMother &&
    left.note === right.note &&
    left.upstreamBaseUrl === right.upstreamBaseUrl &&
    left.localPrimaryLimit === right.localPrimaryLimit &&
    left.localSecondaryLimit === right.localSecondaryLimit &&
    left.localLimitUnit === right.localLimitUnit &&
    left.apiKey === right.apiKey &&
    areAccountDraftTagIdsEqual(left.tagIds, right.tagIds)
  );
}

export function mergeDraftAfterAccountSave(
  current: AccountDraft,
  saveStartedDraft: AccountDraft,
  responseDraft: AccountDraft,
): AccountDraft {
  return {
    displayName:
      current.displayName === saveStartedDraft.displayName
        ? responseDraft.displayName
        : current.displayName,
    groupName:
      current.groupName === saveStartedDraft.groupName
        ? responseDraft.groupName
        : current.groupName,
    isMother:
      current.isMother === saveStartedDraft.isMother
        ? responseDraft.isMother
        : current.isMother,
    note:
      current.note === saveStartedDraft.note
        ? responseDraft.note
        : current.note,
    upstreamBaseUrl:
      current.upstreamBaseUrl === saveStartedDraft.upstreamBaseUrl
        ? responseDraft.upstreamBaseUrl
        : current.upstreamBaseUrl,
    tagIds: areAccountDraftTagIdsEqual(current.tagIds, saveStartedDraft.tagIds)
      ? responseDraft.tagIds
      : current.tagIds,
    localPrimaryLimit:
      current.localPrimaryLimit === saveStartedDraft.localPrimaryLimit
        ? responseDraft.localPrimaryLimit
        : current.localPrimaryLimit,
    localSecondaryLimit:
      current.localSecondaryLimit === saveStartedDraft.localSecondaryLimit
        ? responseDraft.localSecondaryLimit
        : current.localSecondaryLimit,
    localLimitUnit:
      current.localLimitUnit === saveStartedDraft.localLimitUnit
        ? responseDraft.localLimitUnit
        : current.localLimitUnit,
    apiKey:
      current.apiKey === saveStartedDraft.apiKey
        ? responseDraft.apiKey
        : current.apiKey,
  };
}
