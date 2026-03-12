/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { I18nProvider } from '../../i18n'
import UpstreamAccountCreatePage from './UpstreamAccountCreate'

const navigateMock = vi.hoisted(() => vi.fn())
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
}))

vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom')
  return {
    ...actual,
    useNavigate: () => navigateMock,
  }
})

vi.mock('../../hooks/useUpstreamAccounts', () => ({
  useUpstreamAccounts: hookMocks.useUpstreamAccounts,
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

beforeEach(() => {
  vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
    key === 'codex-vibe-monitor.locale' ? 'en' : null,
  )
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  navigateMock.mockReset()
  vi.clearAllMocks()
})

function render(initialEntry = '/account-pool/upstream-accounts/new') {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <I18nProvider>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route path="/account-pool/upstream-accounts/new" element={<UpstreamAccountCreatePage />} />
          </Routes>
        </MemoryRouter>
      </I18nProvider>,
    )
  })
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function setInputValue(selector: string, value: string) {
  const input = host?.querySelector(selector)
  if (!(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)) {
    throw new Error(`missing input: ${selector}`)
  }
  const prototype = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set
  if (!setter) {
    throw new Error(`missing native setter: ${selector}`)
  }
  act(() => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new Event('change', { bubbles: true }))
  })
  return input
}

function clickButton(matcher: RegExp) {
  const button = Array.from(host?.querySelectorAll('button') ?? []).find(
    (candidate) => candidate instanceof HTMLButtonElement && matcher.test(candidate.textContent ?? ''),
  )
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing button: ${matcher}`)
  }
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  return button
}

function getBatchRows() {
  return host?.querySelectorAll('[data-testid^="batch-oauth-row-"]') ?? []
}

function mockUpstreamAccounts(overrides: Partial<ReturnType<typeof hookMocks.useUpstreamAccounts>> = {}) {
  hookMocks.useUpstreamAccounts.mockReturnValue({
    items: [
      {
        id: 5,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Existing OAuth',
        groupName: 'prod',
        status: 'active',
        enabled: true,
      },
    ],
    writesEnabled: true,
    isLoading: false,
    error: null,
    beginOauthLogin: vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    }),
    getLoginSession: vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    }),
    completeOauthLogin: vi.fn().mockResolvedValue({ id: 41, displayName: 'Row One' }),
    createApiKeyAccount: vi.fn(),
    ...overrides,
  })
}

describe('UpstreamAccountCreatePage batch oauth', () => {
  it('opens batch oauth mode from the query string with one empty row', () => {
    mockUpstreamAccounts()
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    expect(Array.from(host?.querySelectorAll('[role="tab"]') ?? []).some((tab) => /Batch OAuth/.test(tab.textContent ?? ''))).toBe(true)
    expect(getBatchRows()).toHaveLength(1)
    expect(host?.textContent).toContain('Batch Codex OAuth onboarding')
  })

  it('forces relink flows back to single oauth mode', () => {
    mockUpstreamAccounts()
    render('/account-pool/upstream-accounts/new?accountId=5&mode=batchOauth')

    expect(host?.textContent).toContain('Re-authorize upstream account')
    expect(host?.textContent).not.toContain('Batch OAuth')

    const displayNameInput = host?.querySelector('input[name="oauthDisplayName"]')
    expect(displayNameInput).toBeInstanceOf(HTMLInputElement)
    expect((displayNameInput as HTMLInputElement).value).toBe('Existing OAuth')
  })

  it('clears a pending row session when metadata changes', async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    })
    mockUpstreamAccounts({ beginOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setInputValue('input[name^="batchOauthDisplayName-"]', 'Row One')
    await flushAsync()
    clickButton(/Generate OAuth URL/i)
    await flushAsync()

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: 'Row One',
      groupName: undefined,
      note: undefined,
    })
    expect(host?.querySelector('input[value^="https://auth.openai.com/authorize"]')).toBeTruthy()

    setInputValue('input[name^="batchOauthNote-"]', 'Needs a new login')

    expect(host?.querySelector('input[value^="https://auth.openai.com/authorize"]')).toBeFalsy()
    expect(host?.textContent).toContain('Metadata changed. Generate a fresh OAuth URL for this row before completing login.')
  })

  it('completes one row without leaving the batch page', async () => {
    const beginOauthLogin = vi
      .fn()
      .mockResolvedValueOnce({
        loginId: 'login-1',
        status: 'pending',
        authUrl: 'https://auth.openai.com/authorize?login=1',
        redirectUri: 'http://localhost:1455/oauth/callback',
        expiresAt: '2026-03-13T10:00:00.000Z',
        accountId: null,
        error: null,
      })
      .mockResolvedValueOnce({
        loginId: 'login-2',
        status: 'pending',
        authUrl: 'https://auth.openai.com/authorize?login=2',
        redirectUri: 'http://localhost:1455/oauth/callback',
        expiresAt: '2026-03-13T10:00:00.000Z',
        accountId: null,
        error: null,
      })
    const completeOauthLogin = vi.fn().mockResolvedValue({ id: 41, displayName: 'Row One' })
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    clickButton(/Add row/i)
    expect(getBatchRows()).toHaveLength(2)

    const displayNames = host?.querySelectorAll('input[name^="batchOauthDisplayName-"]') ?? []
    const inputSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    if (!inputSetter) throw new Error('missing native input setter')
    act(() => {
      inputSetter.call(displayNames[0], 'Row One')
      displayNames[0]?.dispatchEvent(new Event('input', { bubbles: true }))
      displayNames[0]?.dispatchEvent(new Event('change', { bubbles: true }))
      inputSetter.call(displayNames[1], 'Row Two')
      displayNames[1]?.dispatchEvent(new Event('input', { bubbles: true }))
      displayNames[1]?.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await flushAsync()

    let generateButtons = Array.from(host?.querySelectorAll('button') ?? []).filter((button) => /Generate OAuth URL/.test(button.textContent ?? ''))
    act(() => {
      generateButtons[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushAsync()
    generateButtons = Array.from(host?.querySelectorAll('button') ?? []).filter((button) => /Generate OAuth URL|Regenerate OAuth URL/.test(button.textContent ?? ''))
    act(() => {
      generateButtons[1]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushAsync()

    const callbackInputs = host?.querySelectorAll('input[name^="batchOauthCallbackUrl-"]') ?? []
    act(() => {
      inputSetter.call(callbackInputs[0], 'http://localhost:1455/oauth/callback?code=row-one')
      callbackInputs[0]?.dispatchEvent(new Event('input', { bubbles: true }))
      callbackInputs[0]?.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await flushAsync()

    const completeButtons = Array.from(host?.querySelectorAll('button') ?? []).filter((button) => /Complete OAuth login/.test(button.textContent ?? ''))
    act(() => {
      completeButtons[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushAsync()

    expect(completeOauthLogin).toHaveBeenCalledWith('login-1', {
      callbackUrl: 'http://localhost:1455/oauth/callback?code=row-one',
    })
    expect(host?.textContent).toContain('Row One is ready. Continue with the remaining rows when you are done here.')
    expect(getBatchRows()).toHaveLength(2)
    expect(navigateMock).not.toHaveBeenCalled()
  })
})
