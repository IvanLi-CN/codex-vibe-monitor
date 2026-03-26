/** @vitest-environment jsdom */
import { act, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { UpstreamAccountSummary } from '../lib/api'
import { useMotherSwitchNotifications } from './useMotherSwitchNotifications'

const showMotherSwitchUndo = vi.fn()

vi.mock('../components/ui/system-notifications', () => ({
  useSystemNotifications: () => ({
    showMotherSwitchUndo,
  }),
}))

vi.mock('../lib/api', () => ({
  updateUpstreamAccount: vi.fn(),
}))

vi.mock('../lib/upstreamAccountsEvents', () => ({
  emitUpstreamAccountsChanged: vi.fn(),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

function createSummary(
  overrides: Pick<UpstreamAccountSummary, 'id' | 'displayName' | 'groupName' | 'isMother'>,
): UpstreamAccountSummary {
  return {
    id: overrides.id,
    kind: 'oauth_codex',
    provider: 'codex',
    displayName: overrides.displayName,
    groupName: overrides.groupName,
    isMother: overrides.isMother,
    status: 'active',
    enabled: true,
    tags: [],
    effectiveRoutingRule: {} as UpstreamAccountSummary['effectiveRoutingRule'],
  }
}

function HookHarness(props: {
  onReady: (notify: (previousItems: UpstreamAccountSummary[], nextItems: UpstreamAccountSummary[]) => void) => void
}) {
  const notifyMotherSwitches = useMotherSwitchNotifications()

  useEffect(() => {
    props.onReady(notifyMotherSwitches)
  }, [notifyMotherSwitches, props])

  return null
}

function renderHookHarness(
  onReady: (notify: (previousItems: UpstreamAccountSummary[], nextItems: UpstreamAccountSummary[]) => void) => void,
) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(<HookHarness onReady={onReady} />)
  })
}

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  showMotherSwitchUndo.mockReset()
  document.body.innerHTML = ''
})

describe('useMotherSwitchNotifications', () => {
  it('skips notifications when the same mother account only moves to another group', () => {
    let notifyMotherSwitches:
      | ((previousItems: UpstreamAccountSummary[], nextItems: UpstreamAccountSummary[]) => void)
      | null = null
    renderHookHarness((notify) => {
      notifyMotherSwitches = notify
    })

    const previousItems = [
      createSummary({
        id: 18,
        displayName: 'Codex Pro - Tokyo',
        groupName: 'night-ops',
        isMother: true,
      }),
    ]
    const nextItems = [
      createSummary({
        id: 18,
        displayName: 'Codex Pro - Tokyo',
        groupName: 'analytics',
        isMother: true,
      }),
    ]

    act(() => {
      notifyMotherSwitches?.(previousItems, nextItems)
    })

    expect(showMotherSwitchUndo).not.toHaveBeenCalled()
  })

  it('still notifies when moving groups also replaces another mother account', () => {
    let notifyMotherSwitches:
      | ((previousItems: UpstreamAccountSummary[], nextItems: UpstreamAccountSummary[]) => void)
      | null = null
    renderHookHarness((notify) => {
      notifyMotherSwitches = notify
    })

    const previousItems = [
      createSummary({
        id: 18,
        displayName: 'Codex Pro - Tokyo',
        groupName: 'night-ops',
        isMother: true,
      }),
      createSummary({
        id: 33,
        displayName: 'Codex Pro - Analytics',
        groupName: 'analytics',
        isMother: true,
      }),
    ]
    const nextItems = [
      createSummary({
        id: 18,
        displayName: 'Codex Pro - Tokyo',
        groupName: 'analytics',
        isMother: true,
      }),
      createSummary({
        id: 33,
        displayName: 'Codex Pro - Analytics',
        groupName: 'analytics',
        isMother: false,
      }),
    ]

    act(() => {
      notifyMotherSwitches?.(previousItems, nextItems)
    })

    expect(showMotherSwitchUndo).toHaveBeenCalledTimes(1)
    expect(showMotherSwitchUndo.mock.calls[0]?.[0]?.payload).toMatchObject({
      groupKey: 'analytics',
      previousMotherAccountId: 33,
      newMotherAccountId: 18,
    })
  })
})
