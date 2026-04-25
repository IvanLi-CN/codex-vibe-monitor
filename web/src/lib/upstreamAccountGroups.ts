import type { UpstreamAccountGroupSummary } from './api'

export interface UpstreamAccountGroupOption {
  groupName: string
  accountCount?: number
  isPersisted?: boolean
}

export function normalizeGroupName(value?: string | null): string {
  return value?.trim() ?? ''
}

export function buildGroupNoteMap(groups: UpstreamAccountGroupSummary[]): Map<string, string> {
  return new Map(
    groups.map((group) => [normalizeGroupName(group.groupName), group.note ?? '']),
  )
}

export function resolveGroupConcurrencyLimit(
  groups: UpstreamAccountGroupSummary[],
  drafts: Record<string, number>,
  groupName?: string | null,
): number {
  const normalized = normalizeGroupName(groupName)
  if (!normalized) return 0
  const existing = groups.find((group) => normalizeGroupName(group.groupName) === normalized)
  if (existing) return existing.concurrencyLimit ?? 0
  return drafts[normalized] ?? 0
}

export function buildGroupNameSuggestions(
  names: Array<string | null | undefined>,
  groups: UpstreamAccountGroupSummary[],
  drafts: Record<string, string>,
): string[] {
  return buildGroupOptions(names, groups, drafts).map((group) => group.groupName)
}

export function buildGroupOptions(
  names: Array<string | null | undefined>,
  groups: UpstreamAccountGroupSummary[],
  drafts: Record<string, string>,
): UpstreamAccountGroupOption[] {
  const values = new Set<string>()
  const options = new Map<string, UpstreamAccountGroupOption>()

  for (const name of names) {
    const normalized = normalizeGroupName(name)
    if (normalized) {
      values.add(normalized)
    }
  }

  for (const group of groups) {
    const normalized = normalizeGroupName(group.groupName)
    if (normalized) {
      values.add(normalized)
      options.set(normalized, {
        groupName: normalized,
        accountCount: Math.max(0, Math.trunc(group.accountCount ?? 0)),
        isPersisted: true,
      })
    }
  }

  for (const name of Object.keys(drafts)) {
    const normalized = normalizeGroupName(name)
    if (normalized) {
      values.add(normalized)
      options.set(normalized, options.get(normalized) ?? {
        groupName: normalized,
        accountCount: 0,
        isPersisted: false,
      })
    }
  }

  return Array.from(values)
    .sort((left, right) => left.localeCompare(right))
    .map((groupName) => options.get(groupName) ?? {
      groupName,
      accountCount: 0,
      isPersisted: false,
    })
}

export function upsertGroupSummary(
  groups: UpstreamAccountGroupSummary[],
  nextGroup: UpstreamAccountGroupSummary,
): UpstreamAccountGroupSummary[] {
  const normalized = normalizeGroupName(nextGroup.groupName)
  if (!normalized) return groups

  const nextSummary = {
    ...nextGroup,
    groupName: normalized,
  }
  const existingIndex = groups.findIndex(
    (group) => normalizeGroupName(group.groupName) === normalized,
  )

  if (existingIndex >= 0) {
    return groups.map((group, index) => (index === existingIndex ? nextSummary : group))
  }

  return [...groups, nextSummary].sort((left, right) => left.groupName.localeCompare(right.groupName))
}

export function removeGroupSummary(
  groups: UpstreamAccountGroupSummary[],
  groupName?: string | null,
): UpstreamAccountGroupSummary[] {
  const normalized = normalizeGroupName(groupName)
  if (!normalized) return groups
  return groups.filter((group) => normalizeGroupName(group.groupName) !== normalized)
}

export function isExistingGroup(
  groups: UpstreamAccountGroupSummary[],
  groupName?: string | null,
): boolean {
  const normalized = normalizeGroupName(groupName)
  return normalized.length > 0 && groups.some((group) => normalizeGroupName(group.groupName) === normalized)
}

export function resolveGroupNote(
  groups: UpstreamAccountGroupSummary[],
  drafts: Record<string, string>,
  groupName?: string | null,
): string {
  const normalized = normalizeGroupName(groupName)
  if (!normalized) return ''
  const existing = groups.find((group) => normalizeGroupName(group.groupName) === normalized)
  if (existing) return existing.note ?? ''
  return drafts[normalized] ?? ''
}
