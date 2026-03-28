/** @vitest-environment jsdom */
import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import type { ForwardProxyBindingNode } from '../lib/api'
import { UpstreamAccountGroupNoteDialog } from './UpstreamAccountGroupNoteDialog'

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

type DialogProps = ComponentProps<typeof UpstreamAccountGroupNoteDialog>

let host: HTMLDivElement | null = null
let overlayRoot: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  })
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  })
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    writable: true,
    value: () => undefined,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  overlayRoot?.remove()
  host = null
  overlayRoot = null
  root = null
})

function renderDialog(props: Partial<DialogProps> = {}) {
  host = document.createElement('div')
  overlayRoot = document.createElement('div')
  document.body.appendChild(host)
  document.body.appendChild(overlayRoot)
  root = createRoot(host)

  const defaultNodes: ForwardProxyBindingNode[] = [
    {
      key: '__direct__',
      source: 'direct',
      displayName: 'Direct',
      protocolLabel: 'DIRECT',
      penalized: false,
      selectable: true,
      last24h: [],
    },
    {
      key: 'fpn_7f1080a2fdb3a4d1',
      source: 'manual',
      displayName: 'JP Edge 01',
      protocolLabel: 'HTTP',
      penalized: false,
      selectable: true,
      last24h: [],
    },
  ]

  const defaults: DialogProps = {
    open: true,
    container: overlayRoot,
    groupName: 'production',
    note: 'Premium routing',
    existing: true,
    busy: false,
    error: null,
    boundProxyKeys: [],
    availableProxyNodes: defaultNodes,
    onNoteChange: () => undefined,
    onBoundProxyKeysChange: () => undefined,
    onClose: () => undefined,
    onSave: () => undefined,
    title: 'Edit group settings',
    existingDescription: 'Existing group',
    draftDescription: 'Draft group',
    noteLabel: 'Group note',
    notePlaceholder: 'Note',
    cancelLabel: 'Cancel',
    saveLabel: 'Save',
    closeLabel: 'Close',
    existingBadgeLabel: 'Persisted group',
    draftBadgeLabel: 'Draft group',
    proxyBindingsLabel: 'Bound proxy nodes',
    proxyBindingsHint: 'Leave empty to keep automatic routing.',
    proxyBindingsAutomaticLabel: 'No nodes bound. This group uses automatic routing.',
    proxyBindingsEmptyLabel: 'No proxy nodes available.',
    proxyBindingsMissingLabel: 'Missing',
    proxyBindingsUnavailableLabel: 'Unavailable',
    proxyBindingsChartLabel: '24h request trend',
    proxyBindingsChartSuccessLabel: 'Success',
    proxyBindingsChartFailureLabel: 'Failure',
    proxyBindingsChartEmptyLabel: 'No 24h data',
    proxyBindingsChartTotalLabel: 'Total requests',
    proxyBindingsChartAriaLabel: 'Last 24h request volume chart',
    proxyBindingsChartInteractionHint: 'Hover or tap for details.',
    proxyBindingsChartLocaleTag: 'en-US',
  }

  act(() => {
    root?.render(<UpstreamAccountGroupNoteDialog {...defaults} {...props} />)
  })
}

function bodyText() {
  return document.body.textContent ?? ''
}

describe('UpstreamAccountGroupNoteDialog', () => {
  it('shows protocol badges, keeps direct available, and never renders raw subscription URLs', () => {
    renderDialog({
      boundProxyKeys: ['__direct__'],
      groupName: 'latam',
      note: '',
      availableProxyNodes: [
        {
          key: '__direct__',
          source: 'direct',
          displayName: 'Direct',
          protocolLabel: 'DIRECT',
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: 'fpn_vless_stable_key',
          source: 'subscription',
          displayName: 'Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS',
          protocolLabel: 'VLESS',
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: 'fpn_drain_stable_key',
          source: 'manual',
          displayName: 'Drain Node',
          protocolLabel: 'HTTP',
          penalized: true,
          selectable: false,
          last24h: [],
        },
      ],
    })

    const text = bodyText()
    expect(text).toContain('Direct')
    expect(text).toContain('DIRECT')
    expect(text).toContain('VLESS')
    expect(text).not.toContain('vless://')

    const scrollRegion = document.querySelector(
      '[data-testid="proxy-binding-options-scroll-region"]',
    ) as HTMLElement | null
    expect(scrollRegion).not.toBeNull()
    expect(scrollRegion?.className).toContain('overflow-y-auto')

    const dialog = document.querySelector('[role="dialog"]') as HTMLElement | null
    expect(dialog).not.toBeNull()
    expect(dialog?.className).not.toContain('max-w-[72rem]')
    expect(dialog?.className).toContain('sm:max-w-[44rem]')

    const truncatedTitle = document.querySelector(
      '[title="Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS"]',
    ) as HTMLElement | null
    expect(truncatedTitle).not.toBeNull()
    expect(truncatedTitle?.className).toContain('truncate')
  })

  it('adds identity hints for duplicate and missing bindings without exposing stored keys', () => {
    renderDialog({
      groupName: 'overflow',
      note: '',
      boundProxyKeys: ['shared-edge-a', 'legacy-missing-binding'],
      availableProxyNodes: [
        {
          key: 'shared-edge-a',
          source: 'subscription',
          displayName: 'Shared Edge',
          protocolLabel: 'HTTP',
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: 'shared-edge-b',
          source: 'subscription',
          displayName: 'Shared Edge',
          protocolLabel: 'HTTP',
          penalized: false,
          selectable: true,
          last24h: [],
        },
      ],
    })

    const text = bodyText()
    expect(text).not.toContain('legacy-missing-binding')

    const identityHints = Array.from(document.querySelectorAll('[title^="ID "]'))
    expect(identityHints.length).toBeGreaterThanOrEqual(3)
    expect(text).toContain('Missing')
  })

  it('shows visible identity hints for long truncated node names even when labels are unique', () => {
    renderDialog({
      groupName: 'overflow',
      note: '',
      boundProxyKeys: ['edge-long-a'],
      availableProxyNodes: [
        {
          key: 'edge-long-a',
          source: 'subscription',
          displayName: 'ivan-hinet-vless-vision-west-region-priority-a1',
          protocolLabel: 'VLESS',
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: 'edge-long-b',
          source: 'subscription',
          displayName: 'ivan-hinet-vless-vision-west-region-priority-b9',
          protocolLabel: 'VLESS',
          penalized: false,
          selectable: true,
          last24h: [],
        },
      ],
    })

    const identityHints = Array.from(document.querySelectorAll('[title^="ID "]'))
    expect(identityHints.length).toBeGreaterThanOrEqual(2)
  })

  it('renders restored non-ASCII display names for unavailable bound nodes without falling back to raw keys', () => {
    renderDialog({
      boundProxyKeys: ['fpn_deadbeefcafebabe'],
      availableProxyNodes: [
        {
          key: 'fpn_deadbeefcafebabe',
          source: 'missing',
          displayName: '东京专线 A',
          protocolLabel: 'VLESS',
          penalized: false,
          selectable: false,
          last24h: [],
        },
      ],
    })

    expect(bodyText()).toContain('东京专线 A')
    expect(bodyText()).toContain('Unavailable')
    expect(bodyText()).not.toContain('fpn_deadbeefcafebabe')
  })

  it('falls back to the raw key only when no display metadata is available', () => {
    renderDialog({
      boundProxyKeys: ['fpn_missing_only'],
      availableProxyNodes: [],
    })

    expect(bodyText()).toContain('fpn_missing_only')
    expect(bodyText()).toContain('Missing')
  })

  it('hides unrelated stale missing nodes from other groups', () => {
    renderDialog({
      boundProxyKeys: ['fpn_selected_node'],
      availableProxyNodes: [
        {
          key: 'fpn_selected_node',
          source: 'manual',
          displayName: 'JP Edge 01',
          protocolLabel: 'HTTP',
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: 'fpn_other_group_stale',
          source: 'missing',
          displayName: '别组遗留节点',
          protocolLabel: 'UNKNOWN',
          penalized: false,
          selectable: false,
          last24h: [],
        },
      ],
    })

    expect(bodyText()).toContain('JP Edge 01')
    expect(bodyText()).not.toContain('别组遗留节点')
  })
})
