import { describe, expect, it } from 'vitest'
import {
  buildGroupNameSuggestions,
  buildGroupOptions,
  markUpstreamAccountGroupUsed,
  readApiKeyLastGroupName,
  readUpstreamAccountGroupUsage,
  resolveMostRecentlyUsedGroupName,
  UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY,
  upsertGroupSummary,
  writeApiKeyLastGroupName,
  writeUpstreamAccountGroupUsage,
} from './upstreamAccountGroups'

function createMemoryStorage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial))
  return {
    get length() {
      return values.size
    },
    clear: () => values.clear(),
    getItem: (key: string) => values.get(key) ?? null,
    key: (index: number) => Array.from(values.keys())[index] ?? null,
    removeItem: (key: string) => {
      values.delete(key)
    },
    setItem: (key: string, value: string) => {
      values.set(key, value)
    },
  }
}

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

describe('buildGroupOptions', () => {
  it('orders recently used groups first and keeps alphabetical fallback order', () => {
    const options = buildGroupOptions(
      ['alpha', 'gamma'],
      [
        { groupName: 'beta', accountCount: 2 },
        { groupName: 'delta', accountCount: 1 },
      ],
      {},
      {
        beta: 20,
        alpha: 30,
      },
    )

    expect(options.map((option) => option.groupName)).toEqual([
      'alpha',
      'beta',
      'delta',
      'gamma',
    ])
  })

  it('lets a newly selected draft group participate in recent-use ordering', () => {
    const usage = markUpstreamAccountGroupUsed({ alpha: 10 }, ' draft-team ', 40)
    const options = buildGroupOptions(['alpha'], [], { 'draft-team': '' }, usage)

    expect(options.map((option) => option.groupName)).toEqual(['draft-team', 'alpha'])
    expect(resolveMostRecentlyUsedGroupName(options, usage)).toBe('draft-team')
  })
})

describe('local group usage storage', () => {
  it('degrades when window.localStorage access is blocked', () => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'window')
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        get localStorage() {
          throw new Error('storage blocked')
        },
      },
    })

    try {
      expect(readUpstreamAccountGroupUsage()).toEqual({})
      expect(() => writeUpstreamAccountGroupUsage({ alpha: 10 })).not.toThrow()
    } finally {
      if (descriptor) {
        Object.defineProperty(globalThis, 'window', descriptor)
      } else {
        Reflect.deleteProperty(globalThis, 'window')
      }
    }
  })
})

describe('API Key last group storage', () => {
  it('stores and reads the normalized last successful API Key group', () => {
    const storage = createMemoryStorage()

    writeApiKeyLastGroupName(' production ', storage)

    expect(readApiKeyLastGroupName(storage)).toBe('production')
    expect(storage.getItem(UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY)).toBe(
      JSON.stringify({ groupName: 'production' }),
    )
  })

  it('ignores empty, invalid, and blocked API Key group storage values', () => {
    const invalidStorage = createMemoryStorage({
      [UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY]: '{',
    })
    expect(readApiKeyLastGroupName(invalidStorage)).toBe('')

    const emptyStorage = createMemoryStorage()
    writeApiKeyLastGroupName('   ', emptyStorage)
    expect(emptyStorage.getItem(UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY)).toBeNull()

    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'window')
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        get localStorage() {
          throw new Error('storage blocked')
        },
      },
    })

    try {
      expect(readApiKeyLastGroupName()).toBe('')
      expect(() => writeApiKeyLastGroupName('production')).not.toThrow()
    } finally {
      if (descriptor) {
        Object.defineProperty(globalThis, 'window', descriptor)
      } else {
        Reflect.deleteProperty(globalThis, 'window')
      }
    }
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
