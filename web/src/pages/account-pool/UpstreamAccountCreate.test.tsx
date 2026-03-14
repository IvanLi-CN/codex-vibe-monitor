/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { SystemNotificationProvider } from '../../components/ui/system-notifications'
import { I18nProvider } from '../../i18n'
import UpstreamAccountCreatePage from './UpstreamAccountCreate'

const navigateMock = vi.hoisted(() => vi.fn())
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
}))
const apiMocks = vi.hoisted(() => ({
  updateUpstreamAccount: vi.fn().mockResolvedValue({}),
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

vi.mock('../../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../../lib/api')>('../../lib/api')
  return {
    ...actual,
    updateUpstreamAccount: apiMocks.updateUpstreamAccount,
  }
})

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  class ResizeObserverMock {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  })
  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  })
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    writable: true,
    value: vi.fn(),
  })
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
  apiMocks.updateUpstreamAccount.mockReset()
  vi.clearAllMocks()
})

function render(initialEntry = '/account-pool/upstream-accounts/new') {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <I18nProvider>
        <SystemNotificationProvider>
          <MemoryRouter initialEntries={[initialEntry]}>
            <Routes>
              <Route path="/account-pool/upstream-accounts/new" element={<UpstreamAccountCreatePage />} />
            </Routes>
          </MemoryRouter>
        </SystemNotificationProvider>
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
    (candidate) =>
      candidate instanceof HTMLButtonElement
      && matcher.test(candidate.textContent || candidate.getAttribute('aria-label') || candidate.title || ''),
  )
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing button: ${matcher}`)
  }
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  return button
}

function findButton(matcher: RegExp) {
  return Array.from(host?.querySelectorAll('button') ?? []).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement
      && matcher.test(candidate.textContent || candidate.getAttribute('aria-label') || candidate.title || ''),
  ) as HTMLButtonElement | undefined
}

function clickElement(element: Element | null | undefined) {
  if (!(element instanceof HTMLElement)) {
    throw new Error('missing clickable element')
  }
  act(() => {
    element.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
}

function getBatchRows() {
  return host?.querySelectorAll('[data-testid^="batch-oauth-row-"]') ?? []
}


function setComboboxValue(nameSelector: string, value: string) {
  const hiddenInput = host?.querySelector(nameSelector)
  if (!(hiddenInput instanceof HTMLInputElement)) {
    throw new Error(`missing combobox input: ${nameSelector}`)
  }
  const wrapper = hiddenInput.parentElement
  const trigger = wrapper?.querySelector('button[role="combobox"]')
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing combobox trigger: ${nameSelector}`)
  }
  act(() => {
    trigger.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  const searchInput = document.body.querySelector('[cmdk-input]')
  if (!(searchInput instanceof HTMLInputElement)) {
    throw new Error(`missing command input: ${nameSelector}`)
  }
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
  if (!setter) {
    throw new Error(`missing native setter for combobox: ${nameSelector}`)
  }
  act(() => {
    setter.call(searchInput, value)
    searchInput.dispatchEvent(new Event('input', { bubbles: true }))
    searchInput.dispatchEvent(new Event('change', { bubbles: true }))
  })

  const option = Array.from(document.body.querySelectorAll('[cmdk-item]')).find((candidate) =>
    (candidate.textContent || '').includes(value),
  )
  if (!(option instanceof HTMLElement)) {
    throw new Error(`missing combobox option: ${value}`)
  }
  act(() => {
    option.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
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
        isMother: true,
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
    completeOauthLogin: vi.fn().mockResolvedValue({
      id: 41,
      kind: 'oauth_codex',
      provider: 'codex',
      displayName: 'Row One',
      groupName: 'prod',
      isMother: true,
      status: 'active',
      enabled: true,
      history: [],
    }),
    createApiKeyAccount: vi.fn(),
    ...overrides,
  })
}

describe('UpstreamAccountCreatePage batch oauth', () => {
  it('opens batch oauth mode from the query string with five empty rows', () => {
    mockUpstreamAccounts()
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    expect(Array.from(host?.querySelectorAll('[role="tab"]') ?? []).some((tab) => /Batch OAuth/.test(tab.textContent ?? ''))).toBe(true)
    expect(getBatchRows()).toHaveLength(5)
    expect(host?.textContent).toContain('Batch Codex OAuth onboarding')
  })

  it('updates the query-backed mode when tabs change', () => {
    mockUpstreamAccounts()
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    clickButton(/^API key$/i)

    expect(navigateMock).toHaveBeenCalledWith('/account-pool/upstream-accounts/new?mode=apiKey', {
      replace: true,
    })
    expect(host?.textContent).toContain('Codex API key account')
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

  it('applies the header default group to existing blank rows and new rows', async () => {
    mockUpstreamAccounts()
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'prod')
    await flushAsync()

    const groupInputs = Array.from(host?.querySelectorAll('input[name^="batchOauthGroupName-"]') ?? []) as HTMLInputElement[]
    expect(groupInputs[0]?.value).toBe('prod')
    expect(groupInputs[4]?.value).toBe('prod')

    clickButton(/Add row/i)
    await flushAsync()

    const updatedGroupInputs = Array.from(host?.querySelectorAll('input[name^="batchOauthGroupName-"]') ?? []) as HTMLInputElement[]
    expect(updatedGroupInputs[5]?.value).toBe('prod')
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
      isMother: false,
    })
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false)

    clickButton(/Expand note/i)
    setInputValue('input[name^="batchOauthNote-"]', 'Needs a new login')

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true)
    expect(host?.textContent).toContain('Metadata changed. Generate a fresh OAuth URL for this row before completing login.')
  })

  it('invalidates inherited pending sessions when the header default group changes', async () => {
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

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'prod')
    await flushAsync()
    setInputValue('input[name^="batchOauthDisplayName-"]', 'Row One')
    clickButton(/Generate OAuth URL/i)
    await flushAsync()

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'ops')
    await flushAsync()

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true)
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
    const completeOauthLogin = vi.fn().mockResolvedValue({
      id: 41,
      kind: 'oauth_codex',
      provider: 'codex',
      displayName: 'Row One',
      groupName: null,
      isMother: false,
      status: 'active',
      enabled: true,
      history: [],
    })
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    clickButton(/Add row/i)
    expect(getBatchRows()).toHaveLength(6)

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

    let generateButtons = Array.from(host?.querySelectorAll('button') ?? []).filter((button) =>
      /Generate OAuth URL/.test(button.textContent || button.getAttribute('aria-label') || button.getAttribute('title') || ''),
    )
    act(() => {
      generateButtons[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushAsync()
    generateButtons = Array.from(host?.querySelectorAll('button') ?? []).filter((button) =>
      /Generate OAuth URL|Regenerate OAuth URL/.test(
        button.textContent || button.getAttribute('aria-label') || button.getAttribute('title') || '',
      ),
    )
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

    const completeButtons = Array.from(host?.querySelectorAll('button') ?? []).filter((button) =>
      /Complete OAuth login/.test(button.textContent || button.getAttribute('aria-label') || button.getAttribute('title') || ''),
    )
    act(() => {
      completeButtons[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushAsync()

    expect(completeOauthLogin).toHaveBeenCalledWith('login-1', {
      callbackUrl: 'http://localhost:1455/oauth/callback?code=row-one',
    })
    expect(host?.textContent).toContain('Row One is ready. Continue with the remaining rows when you are done here.')
    expect(getBatchRows()).toHaveLength(6)
    expect(navigateMock).not.toHaveBeenCalled()
  })

  it('keeps the mother crown unique within the same draft group', async () => {
    mockUpstreamAccounts()
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'prod')
    await flushAsync()

    const rows = Array.from(getBatchRows())
    const firstToggle = rows[0]?.querySelector('button[aria-label="Toggle mother account"]')
    const secondToggle = rows[1]?.querySelector('button[aria-label="Toggle mother account"]')

    clickElement(firstToggle)
    expect(firstToggle?.getAttribute('aria-pressed')).toBe('true')

    clickElement(secondToggle)
    expect(firstToggle?.getAttribute('aria-pressed')).toBe('false')
    expect(secondToggle?.getAttribute('aria-pressed')).toBe('true')
  })

  it('shows an undo notification after a mother switch and replays the previous owner on undo', async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    })
    const completeOauthLogin = vi.fn().mockResolvedValue({
      id: 41,
      kind: 'oauth_codex',
      provider: 'codex',
      displayName: 'Row One',
      groupName: 'prod',
      isMother: true,
      status: 'active',
      enabled: true,
      history: [],
    })
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setInputValue('input[name^="batchOauthDisplayName-"]', 'Row One')
    setComboboxValue('input[name^="batchOauthGroupName-"]', 'prod')
    await flushAsync()

    const firstRow = Array.from(getBatchRows())[0]
    clickElement(firstRow?.querySelector('button[aria-label="Toggle mother account"]'))
    clickButton(/Generate OAuth URL/i)
    await flushAsync()

    setInputValue('input[name^="batchOauthCallbackUrl-"]', 'http://localhost:1455/oauth/callback?code=row-one')
    clickButton(/Complete OAuth login/i)
    await flushAsync()

    expect(document.body.textContent).toContain('Mother account updated')
    const undoButton = Array.from(document.body.querySelectorAll('button')).find((button) =>
      /^Undo$/i.test(button.textContent || ''),
    )
    clickElement(undoButton)
    await flushAsync()

    expect(apiMocks.updateUpstreamAccount).toHaveBeenCalledWith(5, { isMother: true })
  })
})
