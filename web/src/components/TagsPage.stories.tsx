import { useEffect, useRef, type ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { I18nProvider } from '../i18n'
import type { CreateTagPayload, TagDetail, TagListResponse, TagSummary, UpdateTagPayload } from '../lib/api'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'
import TagsPage from '../pages/account-pool/Tags'

const baseTags: TagSummary[] = [
  {
    id: 1,
    name: 'vip-routing',
    routingRule: {
      guardEnabled: true,
      lookbackHours: 6,
      maxConversations: 4,
      allowCutOut: true,
      allowCutIn: false,
    },
    accountCount: 3,
    groupCount: 2,
    updatedAt: '2026-03-14T09:20:00.000Z',
  },
  {
    id: 2,
    name: 'handoff-blocked',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
    },
    accountCount: 2,
    groupCount: 1,
    updatedAt: '2026-03-14T08:10:00.000Z',
  },
  {
    id: 3,
    name: 'warm-standby',
    routingRule: {
      guardEnabled: true,
      lookbackHours: 2,
      maxConversations: 2,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 1,
    groupCount: 1,
    updatedAt: '2026-03-14T07:40:00.000Z',
  },
]

type Store = {
  writesEnabled: boolean
  tags: TagSummary[]
  nextId: number
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

function toDetail(tag: TagSummary): TagDetail {
  return clone(tag)
}

function jsonResponse(payload: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status,
      headers: { 'Content-Type': 'application/json' },
    }),
  )
}

function parseBody<T>(raw: BodyInit | null | undefined, fallback: T): T {
  if (typeof raw !== 'string' || !raw) return fallback
  try {
    return JSON.parse(raw) as T
  } catch {
    return fallback
  }
}

function applyFilters(tags: TagSummary[], url: URL) {
  const search = (url.searchParams.get('search') || '').trim().toLowerCase()
  const hasAccounts = url.searchParams.get('hasAccounts')
  const guardEnabled = url.searchParams.get('guardEnabled')
  const allowCutOut = url.searchParams.get('allowCutOut')
  const allowCutIn = url.searchParams.get('allowCutIn')
  return tags.filter((tag) => {
    if (search && !tag.name.toLowerCase().includes(search)) return false
    if (hasAccounts === 'true' && tag.accountCount <= 0) return false
    if (guardEnabled === 'true' && !tag.routingRule.guardEnabled) return false
    if (allowCutOut === 'false' && tag.routingRule.allowCutOut !== false) return false
    if (allowCutIn === 'false' && tag.routingRule.allowCutIn !== false) return false
    return true
  })
}

function StorybookTagsMock({ children }: { children: ReactNode }) {
  const storeRef = useRef<Store>({
    writesEnabled: true,
    tags: clone(baseTags),
    nextId: 4,
  })
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const installedRef = useRef(false)

  if (typeof window !== 'undefined' && !installedRef.current) {
    installedRef.current = true
    originalFetchRef.current = window.fetch.bind(window)

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (init?.method || (input instanceof Request ? input.method : 'GET')).toUpperCase()
      const inputUrl = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url
      const url = new URL(inputUrl, window.location.origin)
      const path = url.pathname
      const store = storeRef.current

      if (path === '/api/pool/tags' && method === 'GET') {
        const payload: TagListResponse = {
          writesEnabled: store.writesEnabled,
          items: applyFilters(store.tags, url),
        }
        return jsonResponse(payload)
      }

      if (path === '/api/pool/tags' && method === 'POST') {
        const body = parseBody<CreateTagPayload>(init?.body, {
          name: '',
          guardEnabled: false,
          allowCutIn: true,
          allowCutOut: true,
        } as CreateTagPayload)
        const next: TagSummary = {
          id: store.nextId++,
          name: body.name,
          routingRule: {
            guardEnabled: body.guardEnabled,
            lookbackHours: body.lookbackHours ?? null,
            maxConversations: body.maxConversations ?? null,
            allowCutOut: body.allowCutOut,
            allowCutIn: body.allowCutIn,
          },
          accountCount: 0,
          groupCount: 0,
          updatedAt: '2026-03-14T10:00:00.000Z',
        }
        store.tags = [next, ...store.tags]
        return jsonResponse(next, 201)
      }

      const tagMatch = path.match(/^\/api\/pool\/tags\/(\\d+)$/)
      if (tagMatch && method === 'PATCH') {
        const tagId = Number(tagMatch[1])
        const body = parseBody<UpdateTagPayload>(init?.body, {})
        let updated: TagSummary | null = null
        store.tags = store.tags.map((tag) => {
          if (tag.id !== tagId) return tag
          updated = {
            ...tag,
            name: body.name ?? tag.name,
            routingRule: {
              guardEnabled: body.guardEnabled ?? tag.routingRule.guardEnabled,
              lookbackHours: body.lookbackHours ?? tag.routingRule.lookbackHours ?? null,
              maxConversations: body.maxConversations ?? tag.routingRule.maxConversations ?? null,
              allowCutOut: body.allowCutOut ?? tag.routingRule.allowCutOut,
              allowCutIn: body.allowCutIn ?? tag.routingRule.allowCutIn,
            },
            updatedAt: '2026-03-14T10:20:00.000Z',
          }
          return updated
        })
        return jsonResponse(updated ?? toDetail(store.tags[0]!))
      }

      if (tagMatch && method === 'DELETE') {
        const tagId = Number(tagMatch[1])
        store.tags = store.tags.filter((tag) => tag.id !== tagId)
        return Promise.resolve(new Response(null, { status: 204 }))
      }

      return originalFetchRef.current
        ? originalFetchRef.current(input as Parameters<typeof fetch>[0], init)
        : fetch(input as Parameters<typeof fetch>[0], init)
    }

    window.fetch = mockedFetch
  }

  useEffect(() => {
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current
      }
    }
  }, [])

  return <>{children}</>
}

function TagsPageRouter({ initialEntry = '/account-pool/tags' }: { initialEntry?: string }) {
  return (
    <MemoryRouter initialEntries={[initialEntry]}>
      <Routes>
        <Route path="/account-pool" element={<AccountPoolLayout />}>
          <Route path="tags" element={<TagsPage />} />
        </Route>
      </Routes>
    </MemoryRouter>
  )
}

const meta = {
  title: 'Account Pool/Pages/Tags',
  component: TagsPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookTagsMock>
          <Story />
        </StorybookTagsMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof TagsPage>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => <TagsPageRouter />,
}

export const GuardFilterEnabled: Story = {
  render: () => <TagsPageRouter initialEntry="/account-pool/tags?guardEnabled=true" />,
}
