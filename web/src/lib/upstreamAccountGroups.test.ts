import { describe, expect, it } from 'vitest'
import { buildGroupNameSuggestions, upsertGroupSummary } from './upstreamAccountGroups'

describe('buildGroupNameSuggestions', () => {
  it('includes page draft group names alongside persisted groups and account names', () => {
    expect(
      buildGroupNameSuggestions(
        [' prod ', null, ''],
        [{ groupName: 'shared', note: 'Shared note' }],
        {
          'draft-team': 'Draft note',
          ' prod ': 'Duplicate should normalize',
          '': 'Ignored',
        },
      ),
    ).toEqual(['draft-team', 'prod', 'shared'])
  })
})

describe('upsertGroupSummary', () => {
  it('replaces an existing normalized group entry in place', () => {
    expect(
      upsertGroupSummary(
        [
          { groupName: 'prod', note: 'Old note' },
          { groupName: 'shared', note: 'Shared note' },
        ],
        { groupName: ' prod ', note: 'New note' },
      ),
    ).toEqual([
      { groupName: 'prod', note: 'New note' },
      { groupName: 'shared', note: 'Shared note' },
    ])
  })
})
