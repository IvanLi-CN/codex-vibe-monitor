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
