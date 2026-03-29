import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import type { ForwardProxyBindingNode } from '../lib/api'
import { UpstreamAccountGroupNoteDialog } from './UpstreamAccountGroupNoteDialog'

type DialogHarnessProps = {
  groupName: string
  note: string
  existing: boolean
  busy?: boolean
  error?: string | null
  boundProxyKeys?: string[]
  availableProxyNodes?: ForwardProxyBindingNode[]
}

function buildRequestBuckets(seed: number, baseline: number, failuresEvery: number): ForwardProxyBindingNode['last24h'] {
  const start = Date.parse('2026-03-01T00:00:00.000Z')
  return Array.from({ length: 24 }, (_, index) => {
    const bucketStart = new Date(start + index * 3600_000).toISOString()
    const bucketEnd = new Date(start + (index + 1) * 3600_000).toISOString()
    const successCount = Math.max(0, Math.round(baseline + Math.sin((index + seed) / 2.4) * (baseline * 0.35)))
    const failureCount = index % failuresEvery === 0 ? Math.max(0, Math.round(1 + ((seed + index) % 3))) : 0
    return {
      bucketStart,
      bucketEnd,
      successCount,
      failureCount,
    }
  })
}

const directBindingKey = '__direct__'

const defaultForwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: directBindingKey,
    source: 'direct',
    displayName: 'Direct',
    protocolLabel: 'DIRECT',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(0, 16, 8),
  },
  {
    key: 'fpn_5a7b0c1d2e3f4a10',
    source: 'manual',
    displayName: 'JP Edge 01',
    protocolLabel: 'HTTP',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(1, 18, 7),
  },
  {
    key: 'fpn_8b9c0d1e2f3a4b20',
    source: 'subscription',
    displayName: 'SG Edge 02',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
  {
    key: 'fpn_0c1d2e3f4a5b6c40',
    source: 'subscription',
    displayName: 'US Edge 03',
    protocolLabel: 'VLESS',
    penalized: true,
    selectable: true,
    last24h: buildRequestBuckets(9, 10, 4),
  },
  {
    key: 'fpn_1d2e3f4a5b6c7d50',
    source: 'subscription',
    displayName: 'Ivan-la-vless-vision-01KHTAANPS3QM1DB4H8FEWMYEW',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(10, 9, 4),
  },
  {
    key: 'fpn_2e3f4a5b6c7d8e60',
    source: 'subscription',
    displayName: 'Ivan-hkl-ss2022-01KFXRQH56RQ0SJTYQKS68TCYT',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(12, 10, 6),
  },
  {
    key: 'fpn_3f4a5b6c7d8e9f70',
    source: 'subscription',
    displayName: 'Ivan-iijb-vless-vision-01KKNNTZ3DWEENGMWWF3F9NKT1H',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(13, 8, 5),
  },
  {
    key: 'fpn_4a5b6c7d8e9f0a80',
    source: 'subscription',
    displayName: 'Ivan-ap-ss2022-01KHTAB3M332KVBZ0660GJ2PAR',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(14, 9, 5),
  },
  {
    key: 'fpn_0d1e2f3a4b5c6d30',
    source: 'manual',
    displayName: 'Drain Node',
    protocolLabel: 'HTTP',
    penalized: true,
    selectable: false,
    last24h: buildRequestBuckets(11, 6, 3),
  },
]

const unicodeForwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: 'fpn_13579bdf2468ace0',
    source: 'subscription',
    displayName: '东京专线 A',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(2, 16, 6),
  },
  {
    key: 'fpn_deadbeefcafebabe',
    source: 'missing',
    displayName: '历史东京中继',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: false,
    last24h: [],
  },
]

const refreshedDisplayNameNodes: ForwardProxyBindingNode[] = [
  {
    key: 'fpn_13579bdf2468ace0',
    source: 'subscription',
    displayName: 'Tokyo Edge A (Refreshed Label)',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(4, 15, 8),
  },
  {
    key: 'fpn_8b9c0d1e2f3a4b20',
    source: 'subscription',
    displayName: 'SG Edge 02',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
]

const legacyAliasBindingNodes: ForwardProxyBindingNode[] = [
  {
    key: 'fpn_canonical_vless_key',
    aliasKeys: ['fpn_legacy_vless_alias'],
    source: 'subscription',
    displayName: 'Tokyo Edge A',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(3, 14, 6),
  },
  {
    key: 'fpn_8b9c0d1e2f3a4b20',
    source: 'subscription',
    displayName: 'SG Edge 02',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
]

function DialogHarness({
  note: initialNote,
  boundProxyKeys: initialBoundProxyKeys = [],
  availableProxyNodes = defaultForwardProxyNodes,
  ...args
}: DialogHarnessProps) {
  const [note, setNote] = useState(initialNote)
  const [boundProxyKeys, setBoundProxyKeys] = useState(initialBoundProxyKeys)

  return (
    <div className="min-h-screen bg-base-200 px-6 py-10 text-base-content">
      <div className="mx-auto max-w-3xl rounded-[28px] border border-base-300/70 bg-base-100/80 p-6 shadow-xl backdrop-blur">
        <div className="mb-4 space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/45">
            Shared Group Settings
          </p>
          <h1 className="text-2xl font-semibold">Upstream account group settings dialog</h1>
          <p className="max-w-2xl text-sm leading-6 text-base-content/70">
            This story focuses on the shared group note editor plus hard binding for forward proxy nodes.
          </p>
        </div>
        <UpstreamAccountGroupNoteDialog
          open
          {...args}
          note={note}
          boundProxyKeys={boundProxyKeys}
          availableProxyNodes={availableProxyNodes}
          onNoteChange={setNote}
          onBoundProxyKeysChange={setBoundProxyKeys}
          onClose={() => undefined}
          onSave={() => undefined}
          title="Edit group settings"
          existingDescription="This group already exists. Saving here updates the shared note and proxy bindings immediately."
          draftDescription="This group is not populated yet. Saving here creates its shared settings in advance."
          noteLabel="Group note"
          notePlaceholder="Capture what this group is for, ownership, and any operational caveats."
          cancelLabel="Cancel"
          saveLabel="Save group settings"
          closeLabel="Close dialog"
          existingBadgeLabel="Persisted group"
          draftBadgeLabel="Draft group"
          proxyBindingsLabel="Bound proxy nodes"
          proxyBindingsHint="Leave empty to keep automatic routing. Selected nodes are used as a hard-bound pool for this group."
          proxyBindingsAutomaticLabel="No nodes bound. This group uses automatic routing."
          proxyBindingsEmptyLabel="No proxy nodes available."
          proxyBindingsMissingLabel="Missing"
          proxyBindingsUnavailableLabel="Unavailable"
          proxyBindingsChartLabel="24h request trend"
          proxyBindingsChartSuccessLabel="Success"
          proxyBindingsChartFailureLabel="Failure"
          proxyBindingsChartEmptyLabel="No 24h data"
          proxyBindingsChartTotalLabel="Total requests"
          proxyBindingsChartAriaLabel="Last 24h request volume chart"
          proxyBindingsChartInteractionHint="Hover or tap for details. Focus the chart and use arrow keys to switch points."
          proxyBindingsChartLocaleTag="en-US"
        />
      </div>
    </div>
  )
}

const meta = {
  title: 'Account Pool/Components/Upstream Account Group Settings Dialog',
  component: DialogHarness,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  render: (args) => <DialogHarness {...args} />,
  args: {
    groupName: 'production',
    note: 'Primary team group for premium traffic and shared routing policies.',
    existing: true,
    busy: false,
    error: null,
    boundProxyKeys: [],
    availableProxyNodes: defaultForwardProxyNodes,
  },
} satisfies Meta<typeof DialogHarness>

export default meta

type Story = StoryObj<typeof meta>

export const AutomaticRouting: Story = {}

export const HardBoundMultipleNodes: Story = {
  args: {
    boundProxyKeys: [directBindingKey, 'fpn_5a7b0c1d2e3f4a10', 'fpn_8b9c0d1e2f3a4b20'],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const chart = await canvas.findByLabelText(/JP Edge 01 Last 24h request volume chart/i)
    const firstBar = chart.querySelector('[data-inline-chart-index="0"]')
    if (!(firstBar instanceof HTMLElement)) {
      throw new Error('missing first request trend bar')
    }
    await userEvent.hover(firstBar)
    await expect(within(document.body).getByRole('tooltip')).toBeInTheDocument()
    await expect(within(document.body).getByText(/Success/i)).toBeInTheDocument()
    await expect(within(document.body).getByText(/Failure/i)).toBeInTheDocument()
    await expect(within(document.body).getByText(/Total requests/i)).toBeInTheDocument()
    await expect(canvas.getByText(/^Direct$/i)).toBeInTheDocument()
    await expect(canvas.getByText(/^DIRECT$/i)).toBeInTheDocument()
    await expect(canvas.queryByText(/ss:\/\//i)).not.toBeInTheDocument()
    await expect(canvas.getByTestId('proxy-binding-options-scroll-region').className).toContain('overflow-y-auto')
  },
}

export const NonAsciiBindings: Story = {
  args: {
    groupName: 'apac-premium',
    note: 'Stable keys survive refreshes while operators still see localized display names.',
    boundProxyKeys: ['fpn_13579bdf2468ace0', 'fpn_deadbeefcafebabe'],
    availableProxyNodes: unicodeForwardProxyNodes,
  },
}

export const MissingOrUnavailableBindings: Story = {
  args: {
    groupName: 'overflow',
    note: 'Legacy overflow group with one restored stale binding and one currently unavailable node.',
    boundProxyKeys: ['fpn_0d1e2f3a4b5c6d30', 'fpn_deadbeefcafebabe'],
    availableProxyNodes: [...defaultForwardProxyNodes, unicodeForwardProxyNodes[1]],
  },
}

export const UnavailableOnlyBindingsBlockSave: Story = {
  args: {
    groupName: 'drain-only',
    note: 'This group currently only references unavailable bindings and must not save until one selectable node is chosen.',
    boundProxyKeys: ['fpn_0d1e2f3a4b5c6d30'],
    availableProxyNodes: defaultForwardProxyNodes.filter((node) => node.key === 'fpn_0d1e2f3a4b5c6d30'),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/select at least one available proxy node or clear bindings before saving\./i),
    ).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /save group settings/i })).toBeDisabled()
    await expect(canvas.getByText(/^Unavailable$/i)).toBeInTheDocument()
  },
}

export const RefreshedDisplayNameStableBinding: Story = {
  args: {
    groupName: 'refresh-proof',
    note: 'The stable binding key remains selected after the subscription remark changes.',
    boundProxyKeys: ['fpn_13579bdf2468ace0'],
    availableProxyNodes: refreshedDisplayNameNodes,
  },
}

export const LegacyAliasBindingsRemainSaveable: Story = {
  args: {
    groupName: 'legacy-alias',
    note: 'Groups saved with legacy VLESS aliases still resolve to the current stable node and can be re-saved.',
    boundProxyKeys: ['fpn_legacy_vless_alias'],
    availableProxyNodes: legacyAliasBindingNodes,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(canvas.getByText(/^Tokyo Edge A$/i)).toBeInTheDocument()
    await expect(
      canvas.queryByText(/select at least one available proxy node or clear bindings before saving\./i),
    ).not.toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /save group settings/i })).toBeEnabled()
  },
}
