import type { UpstreamAccountGroupSummary } from './api'

export function normalizeGroupName(value?: string | null): string {
  return value?.trim() ?? ''
}

export function buildGroupNoteMap(groups: UpstreamAccountGroupSummary[]): Map<string, string> {
  return new Map(
    groups.map((group) => [normalizeGroupName(group.groupName), group.note ?? '']),
  )
}

export function buildGroupNameSuggestions(
  names: Array<string | null | undefined>,
  groups: UpstreamAccountGroupSummary[],
  drafts: Record<string, string>,
): string[] {
  const values = new Set<string>()

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
    }
  }

  for (const name of Object.keys(drafts)) {
    const normalized = normalizeGroupName(name)
    if (normalized) {
      values.add(normalized)
    }
  }

  return Array.from(values).sort((left, right) => left.localeCompare(right))
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
