import { describe, expect, it } from 'vitest'
import { buildGroupNameSuggestions } from './upstreamAccountGroups'

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
