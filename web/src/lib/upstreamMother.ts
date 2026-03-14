import type { UpstreamAccountSummary } from './api'

export interface MotherSwitchSnapshot {
  groupKey: string
  groupName: string | null
  newMotherAccountId: number | null
  newMotherDisplayName: string | null
  previousMotherAccountId: number | null
  previousMotherDisplayName: string | null
  hadNoMotherBefore: boolean
}

function normalizeGroupName(groupName?: string | null): string | null {
  const trimmed = groupName?.trim() ?? ''
  return trimmed ? trimmed : null
}

export function normalizeMotherGroupKey(groupName?: string | null): string {
  return normalizeGroupName(groupName) ?? ''
}

export function applyMotherUpdateToItems(
  items: UpstreamAccountSummary[],
  updated: UpstreamAccountSummary,
): UpstreamAccountSummary[] {
  const nextItems = items.map((item) => (item.id === updated.id ? updated : item))
  if (!nextItems.some((item) => item.id === updated.id)) {
    nextItems.unshift(updated)
  }

  if (!updated.isMother) {
    return nextItems
  }

  const groupKey = normalizeMotherGroupKey(updated.groupName)
  return nextItems.map((item) => {
    if (item.id === updated.id) return updated
    if (!item.isMother) return item
    return normalizeMotherGroupKey(item.groupName) === groupKey ? { ...item, isMother: false } : item
  })
}

function buildMotherMap(items: UpstreamAccountSummary[]) {
  const byGroup = new Map<
    string,
    { accountId: number; displayName: string; groupName: string | null }
  >()
  for (const item of items) {
    if (!item.isMother) continue
    byGroup.set(normalizeMotherGroupKey(item.groupName), {
      accountId: item.id,
      displayName: item.displayName,
      groupName: normalizeGroupName(item.groupName),
    })
  }
  return byGroup
}

export function detectMotherSwitches(
  previousItems: UpstreamAccountSummary[],
  nextItems: UpstreamAccountSummary[],
): MotherSwitchSnapshot[] {
  const previous = buildMotherMap(previousItems)
  const next = buildMotherMap(nextItems)
  const groupKeys = new Set<string>([...previous.keys(), ...next.keys()])
  const changes: MotherSwitchSnapshot[] = []

  for (const groupKey of groupKeys) {
    const previousMother = previous.get(groupKey)
    const nextMother = next.get(groupKey)
    if (previousMother?.accountId === nextMother?.accountId) continue

    changes.push({
      groupKey,
      groupName: nextMother?.groupName ?? previousMother?.groupName ?? null,
      newMotherAccountId: nextMother?.accountId ?? null,
      newMotherDisplayName: nextMother?.displayName ?? null,
      previousMotherAccountId: previousMother?.accountId ?? null,
      previousMotherDisplayName: previousMother?.displayName ?? null,
      hadNoMotherBefore: previousMother == null,
    })
  }

  return changes
}
