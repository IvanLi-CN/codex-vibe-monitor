import type { UpstreamAccountGroupSummary } from './api'

export interface UpstreamAccountGroupOption {
  groupName: string
  accountCount?: number
  isPersisted?: boolean
}

export type UpstreamAccountGroupUsageMap = Record<string, number>

export const UPSTREAM_ACCOUNT_CREATE_GROUP_USAGE_STORAGE_KEY =
  'codex-vibe-monitor.account-pool.create.group-usage'

export function normalizeGroupName(value?: string | null): string {
  return value?.trim() ?? ''
}

export function readUpstreamAccountGroupUsage(
  storage?: Storage,
): UpstreamAccountGroupUsageMap {
  try {
    const resolvedStorage =
      storage ?? (typeof window === 'undefined' ? undefined : window.localStorage)
    if (!resolvedStorage) return {}
    const raw = resolvedStorage.getItem(UPSTREAM_ACCOUNT_CREATE_GROUP_USAGE_STORAGE_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {}
    const usage: UpstreamAccountGroupUsageMap = {}
    for (const [groupName, usedAt] of Object.entries(parsed)) {
      const normalized = normalizeGroupName(groupName)
      if (!normalized || typeof usedAt !== 'number' || !Number.isFinite(usedAt)) continue
      usage[normalized] = usedAt
    }
    return usage
  } catch {
    return {}
  }
}

export function writeUpstreamAccountGroupUsage(
  usage: UpstreamAccountGroupUsageMap,
  storage?: Storage,
) {
  try {
    const resolvedStorage =
      storage ?? (typeof window === 'undefined' ? undefined : window.localStorage)
    if (!resolvedStorage) return
    resolvedStorage.setItem(UPSTREAM_ACCOUNT_CREATE_GROUP_USAGE_STORAGE_KEY, JSON.stringify(usage))
  } catch {
    // Ignore storage quota/privacy failures; group memory is only a preference.
  }
}

export function markUpstreamAccountGroupUsed(
  usage: UpstreamAccountGroupUsageMap,
  groupName?: string | null,
  usedAt = Date.now(),
): UpstreamAccountGroupUsageMap {
  const normalized = normalizeGroupName(groupName)
  if (!normalized) return usage
  return {
    ...usage,
    [normalized]: usedAt,
  }
}

export function resolveMostRecentlyUsedGroupName(
  options: UpstreamAccountGroupOption[],
  usage: UpstreamAccountGroupUsageMap,
): string {
  return options.reduce<{ groupName: string; usedAt: number }>(
    (current, option) => {
      const normalized = normalizeGroupName(option.groupName)
      const usedAt = normalized ? usage[normalized] : undefined
      if (typeof usedAt !== 'number' || !Number.isFinite(usedAt)) return current
      if (!current.groupName || usedAt > current.usedAt) {
        return { groupName: normalized, usedAt }
      }
      if (usedAt === current.usedAt && normalized.localeCompare(current.groupName) < 0) {
        return { groupName: normalized, usedAt }
      }
      return current
    },
    { groupName: '', usedAt: Number.NEGATIVE_INFINITY },
  ).groupName
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
  usage: UpstreamAccountGroupUsageMap = {},
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
    .sort((left, right) => {
      const leftUsedAt = usage[left]
      const rightUsedAt = usage[right]
      const leftHasUsage = typeof leftUsedAt === 'number' && Number.isFinite(leftUsedAt)
      const rightHasUsage = typeof rightUsedAt === 'number' && Number.isFinite(rightUsedAt)
      if (leftHasUsage && rightHasUsage && leftUsedAt !== rightUsedAt) {
        return rightUsedAt - leftUsedAt
      }
      if (leftHasUsage !== rightHasUsage) {
        return leftHasUsage ? -1 : 1
      }
      return left.localeCompare(right)
    })
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
