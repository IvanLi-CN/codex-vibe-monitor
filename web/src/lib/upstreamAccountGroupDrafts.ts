export function resolvePersistedGroupNodeShuntEnabled(
  hasDraftNodeShuntEnabled: boolean,
  draftNodeShuntEnabled: boolean | undefined,
  currentNodeShuntEnabled: boolean,
) {
  if (hasDraftNodeShuntEnabled) {
    return draftNodeShuntEnabled === true;
  }
  return currentNodeShuntEnabled;
}

export function resolvePersistedGroupSingleAccountRotationEnabled(
  hasDraftSingleAccountRotationEnabled: boolean,
  draftSingleAccountRotationEnabled: boolean | undefined,
  currentSingleAccountRotationEnabled: boolean,
) {
  if (hasDraftSingleAccountRotationEnabled) {
    return draftSingleAccountRotationEnabled === true;
  }
  return currentSingleAccountRotationEnabled;
}
