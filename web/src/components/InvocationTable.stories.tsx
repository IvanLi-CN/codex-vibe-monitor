import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import { InvocationTable } from './InvocationTable'
import type { ApiInvocation } from '../lib/api'
import { STORYBOOK_COLOR_CONTRAST_TODO } from '../storybook/a11y'

const baseOccurredAt = '2026-02-25T10:15:30Z'
const LONG_PROXY_NAME = 'ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD'

const records: ApiInvocation[] = [
  {
    id: 1001,
    invokeId: 'inv_01JSX0PQ3Z8CFQ7AJK8XEH2N4D',
    occurredAt: baseOccurredAt,
    createdAt: baseOccurredAt,
    source: 'proxy',
    proxyDisplayName: 'Tokyo-Edge-1',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 1632,
    outputTokens: 298,
    cacheInputTokens: 1240,
    reasoningTokens: 84,
    reasoningEffort: 'high',
    totalTokens: 1930,
    cost: 0.0037,
    requesterIp: '203.0.113.42',
    promptCacheKey: 'pck_6f35b9b20f0348af',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    proxyWeightDelta: 0.55,
    tReqReadMs: 1.8,
    tReqParseMs: 3.2,
    tUpstreamConnectMs: 26.1,
    tUpstreamTtfbMs: 184.7,
    tUpstreamStreamMs: 641.9,
    tRespParseMs: 8.6,
    tPersistMs: 2.1,
    tTotalMs: 870.4,
    priceVersion: '2026-02',
  },
  {
    id: 1002,
    invokeId: 'inv_01JSX0Q6YHBFTDVMC3N5NF13R7',
    occurredAt: '2026-02-25T10:18:11Z',
    createdAt: '2026-02-25T10:18:11Z',
    source: 'proxy',
    proxyDisplayName: LONG_PROXY_NAME,
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'failed',
    inputTokens: 884,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningEffort: 'medium',
    totalTokens: 884,
    errorMessage: 'upstream timeout while waiting first byte',
    failureKind: 'upstream_timeout',
    requestedServiceTier: 'priority',
    serviceTier: 'auto',
    proxyWeightDelta: -0.68,
    tReqReadMs: 1.1,
    tReqParseMs: 2.3,
    tUpstreamConnectMs: 48.5,
    tUpstreamTtfbMs: null,
    tUpstreamStreamMs: null,
    tRespParseMs: null,
    tPersistMs: 1.9,
    tTotalMs: 30015.7,
  },
  {
    id: 1003,
    invokeId: 'inv_01JSX0R9N0F2V8G54T5PG17WQH',
    occurredAt: '2026-02-25T10:22:48Z',
    createdAt: '2026-02-25T10:22:48Z',
    source: 'proxy',
    proxyDisplayName: 'Seoul-Edge-2',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    inputTokens: 1520,
    outputTokens: 212,
    cacheInputTokens: 740,
    totalTokens: 1732,
    cost: 0.0051,
    requesterIp: '203.0.113.77',
    promptCacheKey: 'pck_82c89c811a',
    requestedServiceTier: 'priority',
    proxyWeightDelta: 0,
    tReqReadMs: 1.4,
    tReqParseMs: 2.8,
    tUpstreamConnectMs: 31.2,
    tUpstreamTtfbMs: 166.1,
    tUpstreamStreamMs: 512.4,
    tRespParseMs: 5.6,
    tPersistMs: 1.8,
    tTotalMs: 721.3,
  },
]


const fastIndicatorRecords: ApiInvocation[] = [
  {
    id: 1101,
    invokeId: 'inv_fast_effective',
    occurredAt: '2026-02-25T10:30:00Z',
    createdAt: '2026-02-25T10:30:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-effective',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    requestedServiceTier: 'priority',
    serviceTier: 'priority',
    inputTokens: 1200,
    outputTokens: 240,
    totalTokens: 1440,
    cost: 0.0032,
    tUpstreamTtfbMs: 118.3,
    tTotalMs: 640.2,
  },
  {
    id: 1102,
    invokeId: 'inv_fast_requested_auto',
    occurredAt: '2026-02-25T10:31:00Z',
    createdAt: '2026-02-25T10:31:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-requested-auto',
    endpoint: '/v1/responses',
    model: 'gpt-5',
    status: 'failed',
    requestedServiceTier: 'priority',
    serviceTier: 'auto',
    inputTokens: 980,
    outputTokens: 0,
    totalTokens: 980,
    errorMessage: 'upstream timeout while waiting first byte',
    tUpstreamTtfbMs: null,
    tTotalMs: 30010.5,
  },
  {
    id: 1103,
    invokeId: 'inv_fast_requested_missing',
    occurredAt: '2026-02-25T10:32:00Z',
    createdAt: '2026-02-25T10:32:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-requested-missing',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    requestedServiceTier: 'priority',
    inputTokens: 1024,
    outputTokens: 196,
    totalTokens: 1220,
    cost: 0.0038,
    tUpstreamTtfbMs: 142.6,
    tTotalMs: 702.1,
  },
  {
    id: 1104,
    invokeId: 'inv_fast_effective_auto_request',
    occurredAt: '2026-02-25T10:33:00Z',
    createdAt: '2026-02-25T10:33:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-effective-auto-request',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    requestedServiceTier: 'auto',
    serviceTier: 'priority',
    inputTokens: 1188,
    outputTokens: 202,
    totalTokens: 1390,
    cost: 0.0041,
    tUpstreamTtfbMs: 104.4,
    tTotalMs: 611.9,
  },
  {
    id: 1105,
    invokeId: 'inv_fast_none_flex',
    occurredAt: '2026-02-25T10:34:00Z',
    createdAt: '2026-02-25T10:34:00Z',
    source: 'proxy',
    proxyDisplayName: 'Fast-none-flex',
    endpoint: '/v1/responses',
    model: 'gpt-5.4',
    status: 'success',
    requestedServiceTier: 'flex',
    serviceTier: 'flex',
    inputTokens: 1160,
    outputTokens: 188,
    totalTokens: 1348,
    cost: 0.0035,
    tUpstreamTtfbMs: 156.8,
    tTotalMs: 734.7,
  },
]

const reasoningEffortRecords: ApiInvocation[] = [
  {
    id: 2001,
    invokeId: 'inv_reasoning_none',
    occurredAt: '2026-02-25T11:00:00Z',
    createdAt: '2026-02-25T11:00:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-none',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5.1',
    status: 'success',
    inputTokens: 640,
    outputTokens: 112,
    cacheInputTokens: 0,
    reasoningEffort: 'none',
    reasoningTokens: 0,
    totalTokens: 752,
    cost: 0.0018,
    tUpstreamTtfbMs: 96.4,
    tTotalMs: 411.7,
  },
  {
    id: 2002,
    invokeId: 'inv_reasoning_minimal',
    occurredAt: '2026-02-25T11:02:00Z',
    createdAt: '2026-02-25T11:02:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-minimal',
    endpoint: '/v1/responses',
    model: 'gpt-5',
    status: 'success',
    inputTokens: 712,
    outputTokens: 144,
    cacheInputTokens: 128,
    reasoningEffort: 'minimal',
    reasoningTokens: 12,
    totalTokens: 856,
    cost: 0.0021,
    tUpstreamTtfbMs: 118.1,
    tTotalMs: 588.2,
  },
  {
    id: 2003,
    invokeId: 'inv_reasoning_low',
    occurredAt: '2026-02-25T11:04:00Z',
    createdAt: '2026-02-25T11:04:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-low',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 804,
    outputTokens: 166,
    cacheInputTokens: 256,
    reasoningEffort: 'low',
    reasoningTokens: 28,
    totalTokens: 970,
    cost: 0.0024,
    tUpstreamTtfbMs: 132.5,
    tTotalMs: 710.4,
  },
  {
    id: 2004,
    invokeId: 'inv_reasoning_medium',
    occurredAt: '2026-02-25T11:06:00Z',
    createdAt: '2026-02-25T11:06:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-medium',
    endpoint: '/v1/chat/completions',
    model: 'gpt-5',
    status: 'failed',
    inputTokens: 920,
    outputTokens: 0,
    cacheInputTokens: 0,
    reasoningEffort: 'medium',
    totalTokens: 920,
    errorMessage: 'upstream timeout while waiting first byte',
    failureKind: 'upstream_timeout',
    tUpstreamTtfbMs: null,
    tTotalMs: 30012.0,
  },
  {
    id: 2005,
    invokeId: 'inv_reasoning_high',
    occurredAt: '2026-02-25T11:08:00Z',
    createdAt: '2026-02-25T11:08:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-high',
    endpoint: '/v1/responses',
    model: 'gpt-5',
    status: 'success',
    inputTokens: 1012,
    outputTokens: 244,
    cacheInputTokens: 320,
    reasoningEffort: 'high',
    reasoningTokens: 84,
    totalTokens: 1256,
    cost: 0.0031,
    tUpstreamTtfbMs: 188.4,
    tTotalMs: 962.6,
  },
  {
    id: 2006,
    invokeId: 'inv_reasoning_xhigh',
    occurredAt: '2026-02-25T11:10:00Z',
    createdAt: '2026-02-25T11:10:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-xhigh',
    endpoint: '/v1/responses',
    model: 'gpt-5.2',
    status: 'success',
    inputTokens: 1130,
    outputTokens: 318,
    cacheInputTokens: 512,
    reasoningEffort: 'xhigh',
    reasoningTokens: 146,
    totalTokens: 1448,
    cost: 0.0048,
    tUpstreamTtfbMs: 261.3,
    tTotalMs: 1384.9,
  },
  {
    id: 2007,
    invokeId: 'inv_reasoning_missing',
    occurredAt: '2026-02-25T11:12:00Z',
    createdAt: '2026-02-25T11:12:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-missing',
    endpoint: '/v1/responses',
    model: 'gpt-5-mini',
    status: 'success',
    inputTokens: 540,
    outputTokens: 90,
    cacheInputTokens: 64,
    totalTokens: 630,
    cost: 0.0015,
    tUpstreamTtfbMs: 104.7,
    tTotalMs: 498.5,
  },
  {
    id: 2008,
    invokeId: 'inv_reasoning_unknown',
    occurredAt: '2026-02-25T11:14:00Z',
    createdAt: '2026-02-25T11:14:00Z',
    source: 'proxy',
    proxyDisplayName: 'Reasoning-unknown',
    endpoint: '/v1/responses',
    model: 'custom-reasoning-model',
    status: 'success',
    inputTokens: 600,
    outputTokens: 120,
    cacheInputTokens: 0,
    reasoningEffort: 'custom-tier',
    reasoningTokens: 33,
    totalTokens: 720,
    cost: 0.0019,
    tUpstreamTtfbMs: 124.2,
    tTotalMs: 544.0,
  },
]

const meta = {
  title: 'Monitoring/InvocationTable',
  component: InvocationTable,
  tags: ['autodocs'],
  parameters: {
    ...STORYBOOK_COLOR_CONTRAST_TODO,
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          'Shows recent invocation records with status, cost, token usage, proxy metadata, and expandable request details. The default story includes both a `/v1/responses` record with `reasoningTokens` and a `/v1/chat/completions` record that falls back to `—` when `reasoningTokens` is absent. The output summary shows output tokens on the first line and the reasoning-token breakdown on the second line.\n\nVisible reasoning effort cases in this component: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, missing (`—`), and unknown raw strings such as `custom-tier`. The component only shows explicitly recorded request values and does not infer model defaults. According to the OpenAI API docs as checked on 2026-03-07, the general API-level values are `none`, `minimal`, `low`, `medium`, `high`, and `xhigh`, but model support is narrower for some models.\n\nReasoning-effort colors now follow a stable ladder: `none` stays neutral, `minimal/low` use cool informational tones, `medium` moves into the primary tier, `high` warns in amber, `xhigh` escalates to error red, and unknown raw strings use a dashed neutral badge so they cannot be mistaken for a standard level.\n\nUse this component to verify the summary row layout on desktop, the card layout on mobile, and the expanded detail section for request metadata, timing stages, reasoning effort, and reasoning tokens.',
      },
    },
  },
  argTypes: {
    records: {
      control: 'object',
      description:
        'Invocation rows rendered by the table. Include `reasoningEffort` to show the summary badge and `reasoningTokens` to populate both the output-column breakdown and the expanded detail field; missing values render as `—`.',
      table: {
        type: { summary: 'ApiInvocation[]' },
      },
    },
    isLoading: {
      control: 'boolean',
      description: 'Displays the loading spinner state while the table is waiting for records.',
      table: {
        type: { summary: 'boolean' },
        defaultValue: { summary: 'false' },
      },
    },
    error: {
      control: 'text',
      description: 'Optional request error message rendered above the table when loading fails.',
      table: {
        type: { summary: 'string | null' },
        defaultValue: { summary: 'null' },
      },
    },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div data-theme="light" className="bg-base-200 px-6 py-6 text-base-content">
          <div className="mx-auto w-full max-w-6xl p-6">
            <section className="card bg-base-100 shadow-sm">
              <div className="card-body gap-4">
                <Story />
              </div>
            </section>
          </div>
        </div>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationTable>

export default meta

type Story = StoryObj<typeof meta>

const defaultArgs: Story['args'] = {
  records,
  isLoading: false,
  error: null,
}

export const Default: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          'Reference state with two proxy invocations. The output summary shows output tokens with a second-line reasoning-token breakdown; expand a row to inspect the same `reasoningEffort` and `reasoningTokens` in the detail grid.',
      },
    },
  },
}


export const FastIndicatorStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          'Covers the fast indicator matrix: effective priority, requested-only fallback, requested priority with missing response tier, effective priority despite non-priority request, and a flex request with no lightning icon.',
      },
    },
  },
  args: {
    records: fastIndicatorRecords,
    isLoading: false,
    error: null,
  },
}

export const ReasoningEffortStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          'Matrix story for visually checking every reasoning effort state the table may show: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, missing (`—`), and an unknown raw string. Supported API-level values were verified against the OpenAI API docs on 2026-03-07; actual model support remains model-dependent. The intended color ladder is neutral -> cool -> primary -> warning -> error, with unknown values rendered as dashed neutral badges.',
      },
    },
  },
  args: {
    records: reasoningEffortRecords,
    isLoading: false,
    error: null,
  },
}

export const Empty: Story = {
  parameters: {
    docs: {
      description: {
        story: 'Empty state used when the request succeeds but no invocations match the current filters.',
      },
    },
  },
  args: {
    records: [],
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  parameters: {
    docs: {
      description: {
        story: 'Loading placeholder used while invocation records are being fetched or refreshed.',
      },
    },
  },
  args: {
    records: [],
    isLoading: true,
    error: null,
  },
}

export const LoadError: Story = {
  parameters: {
    docs: {
      description: {
        story: 'Error banner state used when the invocation request fails and the user needs retry context.',
      },
    },
  },
  args: {
    records: [],
    isLoading: false,
    error: 'Request failed: 500 Internal Server Error',
  },
}
