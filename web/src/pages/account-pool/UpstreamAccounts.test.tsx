/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { SystemNotificationProvider } from '../../components/ui/system-notifications'
import { I18nProvider } from '../../i18n'
import UpstreamAccountsPage from './UpstreamAccounts'

const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
  useUpstreamStickyConversations: vi.fn(),
}))

vi.mock('../../hooks/useUpstreamAccounts', () => ({
  useUpstreamAccounts: hookMocks.useUpstreamAccounts,
}))

vi.mock('../../hooks/useUpstreamStickyConversations', () => ({
  useUpstreamStickyConversations: hookMocks.useUpstreamStickyConversations,
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: {
      getItem: vi.fn((key: string) => (key === 'codex-vibe-monitor.locale' ? 'en' : null)),
      setItem: vi.fn(),
      removeItem: vi.fn(),
    },
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  vi.clearAllMocks()
})

function render() {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <I18nProvider>
        <SystemNotificationProvider>
          <MemoryRouter initialEntries={['/account-pool/upstream-accounts']}>
            <Routes>
              <Route path="/account-pool/upstream-accounts" element={<UpstreamAccountsPage />} />
            </Routes>
          </MemoryRouter>
        </SystemNotificationProvider>
      </I18nProvider>,
    )
  })
}

function clickByText(pattern: RegExp) {
  const button = Array.from(document.body.querySelectorAll('button')).find((candidate) =>
    pattern.test(candidate.textContent || candidate.getAttribute('aria-label') || ''),
  )
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing button: ${pattern}`)
  }
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

describe('UpstreamAccountsPage mother account editing', () => {
  it('shows the crown badge and emits an undo notification after saving a new mother account', async () => {
    const saveAccount = vi.fn().mockResolvedValue({
      id: 5,
      kind: 'oauth_codex',
      provider: 'codex',
      displayName: 'Existing OAuth',
      groupName: 'prod',
      isMother: true,
      status: 'active',
      enabled: true,
      history: [],
      note: null,
    })

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 5,
          kind: 'oauth_codex',
          provider: 'codex',
          displayName: 'Existing OAuth',
          groupName: 'prod',
          isMother: false,
          status: 'active',
          enabled: true,
        },
        {
          id: 6,
          kind: 'oauth_codex',
          provider: 'codex',
          displayName: 'Current Mother',
          groupName: 'prod',
          isMother: true,
          status: 'active',
          enabled: true,
        },
      ],
      writesEnabled: true,
      selectedId: 5,
      selectedSummary: {
        id: 5,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Existing OAuth',
        groupName: 'prod',
        isMother: false,
        status: 'active',
        enabled: true,
      },
      detail: {
        id: 5,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Existing OAuth',
        groupName: 'prod',
        isMother: false,
        status: 'active',
        enabled: true,
        history: [],
        note: null,
      },
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      saveAccount,
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: 'pool-live••••' },
      saveRouting: vi.fn(),
    })
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: '', rangeEnd: '' },
      isLoading: false,
      error: null,
    })

    render()
    expect(document.body.textContent).toContain('Current Mother')

    clickByText(/Open details/i)
    clickByText(/Use as mother account/i)
    clickByText(/Save changes/i)
    await flushAsync()

    expect(saveAccount).toHaveBeenCalledWith(
      5,
      expect.objectContaining({
        isMother: true,
      }),
    )
    expect(document.body.textContent).toContain('Mother account updated')
  })
})
