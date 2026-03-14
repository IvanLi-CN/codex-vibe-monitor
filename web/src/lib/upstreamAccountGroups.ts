import type { UpstreamAccountGroupSummary } from './api'

export function normalizeGroupName(value?: string | null): string {
  return value?.trim() ?? ''
}

export function buildGroupNoteMap(groups: UpstreamAccountGroupSummary[]): Map<string, string> {
  return new Map(
    groups.map((group) => [normalizeGroupName(group.groupName), group.note ?? '']),
  )
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
