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
const subscriptionSsKey =
  'ss://2022-blake3-aes-128-gcm:EOYQdB4zxDFr9WNrv8HiXg%3D%3D%3A%2FnzEl7kJLV8e@example-hk-01.707979.xyz:443#Ivan-hinet-ss2022-01KF87EBR50MM9JKM9R9BCA9WZ'
const subscriptionVlessKey =
  'vless://e8d10b05-aec8-4cee-be7d-2f5eee61b0a7@hinet-ep.707979.xyz:53842?encryption=none&security=reality&type=tcp&sni=skypapi.onedrive.com&fp=chrome&pbk=abc123&sid=long-subscription-node#Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS'

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
    key: 'jp-edge-01',
    source: 'manual',
    displayName: 'JP Edge 01',
    protocolLabel: 'HTTP',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(1, 18, 7),
  },
  {
    key: subscriptionSsKey,
    source: 'subscription',
    displayName: 'Ivan-hinet-ss2022-01KF87EBR50MM9JKM9R9BCA9WZ',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
  {
    key: subscriptionVlessKey,
    source: 'subscription',
    displayName: 'Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(8, 11, 4),
  },
  {
    key: 'us-edge-03',
    source: 'subscription',
    displayName: 'US Edge 03',
    protocolLabel: 'VLESS',
    penalized: true,
    selectable: true,
    last24h: buildRequestBuckets(9, 10, 4),
  },
  {
    key: 'la-edge-04',
    source: 'subscription',
    displayName: 'Ivan-la-vless-vision-01KHTAANPS3QM1DB4H8FEWMYEW',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(10, 9, 4),
  },
  {
    key: 'hk-edge-05',
    source: 'subscription',
    displayName: 'Ivan-hkl-ss2022-01KFXRQH56RQ0SJTYQKS68TCYT',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(12, 10, 6),
  },
  {
    key: 'ii-edge-06',
    source: 'subscription',
    displayName: 'Ivan-iijb-vless-vision-01KKNNTZ3DWEENGMWWF3F9NKT1H',
    protocolLabel: 'VLESS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(13, 8, 5),
  },
  {
    key: 'ap-edge-07',
    source: 'subscription',
    displayName: 'Ivan-ap-ss2022-01KHTAB3M332KVBZ0660GJ2PAR',
    protocolLabel: 'SS',
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(14, 9, 5),
  },
  {
    key: 'drain-node',
    source: 'manual',
    displayName: 'Drain Node',
    protocolLabel: 'HTTP',
    penalized: true,
    selectable: false,
    last24h: buildRequestBuckets(11, 6, 3),
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
    boundProxyKeys: [directBindingKey, subscriptionVlessKey],
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

export const MissingOrUnavailableBindings: Story = {
  args: {
    groupName: 'overflow',
    note: 'Legacy overflow group with one stale node reference.',
    boundProxyKeys: ['drain-node', 'missing-node-legacy'],
  },
}
