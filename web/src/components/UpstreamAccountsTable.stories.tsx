import type { Meta, StoryObj } from '@storybook/react-vite'
import type { AccountTagSummary, EffectiveRoutingRule, UpstreamAccountSummary } from '../lib/api'
import { UpstreamAccountsTable } from './UpstreamAccountsTable'

const now = '2026-03-11T12:30:00.000Z'
const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
}

const rosterTags: AccountTagSummary[] = [
  {
    id: 1,
    name: 'vip',
    routingRule: defaultEffectiveRoutingRule,
  },
  {
    id: 2,
    name: 'burst-safe',
    routingRule: defaultEffectiveRoutingRule,
  },
  {
    id: 3,
    name: 'prod-apac',
    routingRule: defaultEffectiveRoutingRule,
  },
  {
    id: 4,
    name: 'sticky-pool',
    routingRule: defaultEffectiveRoutingRule,
  },
]

function usage(
  requestCount: number,
  totalTokens: number,
  totalCost: number,
  inputTokens: number,
  outputTokens: number,
  cacheInputTokens: number,
) {
  return {
    requestCount,
    totalTokens,
    totalCost,
    inputTokens,
    outputTokens,
    cacheInputTokens,
  }
}

const items: UpstreamAccountSummary[] = [
  {
    id: 11,
    kind: 'oauth_codex',
    provider: 'codex',
    displayName: 'Codex Pro - Tokyo',
    groupName: 'production',
    isMother: true,
    status: 'active',
    displayStatus: 'active',
    enabled: true,
    enableStatus: 'enabled',
    workStatus: 'working',
    healthStatus: 'normal',
    syncState: 'idle',
    email: 'tokyo@example.com',
    chatgptAccountId: 'org_tokyo',
    planType: 'pro',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
    lastActivityAt: '2026-03-11T12:12:00.000Z',
    activeConversationCount: 3,
    lastAction: 'route_hard_unavailable',
    lastActionSource: 'call',
    lastActionReasonCode: 'upstream_http_429_quota_exhausted',
    lastActionReasonMessage: 'Weekly cap exhausted for this account',
    lastActionHttpStatus: 429,
    lastActionAt: '2026-03-11T12:14:00.000Z',
    primaryWindow: {
      usedPercent: 42,
      usedText: '42% used',
      limitText: '5h rolling window',
      resetsAt: '2026-03-11T14:00:00.000Z',
      windowDurationMins: 300,
      actualUsage: usage(19, 48210, 0.4284, 28140, 16410, 3660),
    },
    secondaryWindow: {
      usedPercent: 18,
      usedText: '18% used',
      limitText: '7d rolling window',
      resetsAt: '2026-03-14T00:00:00.000Z',
      windowDurationMins: 10080,
      actualUsage: usage(73, 182340, 1.6234, 103220, 67480, 11640),
    },
    credits: {
      hasCredits: true,
      unlimited: false,
      balance: '12.80',
    },
    tags: rosterTags,
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: 'requests',
    },
  },
  {
    id: 12,
    kind: 'api_key_codex',
    provider: 'codex',
    displayName: 'Team key - staging with an intentionally long roster label',
    groupName: 'staging',
    isMother: false,
    status: 'active',
    displayStatus: 'active',
    enabled: true,
    enableStatus: 'enabled',
    workStatus: 'rate_limited',
    healthStatus: 'normal',
    syncState: 'idle',
    maskedApiKey: 'sk-live••••••c9f2',
    lastSyncedAt: '2026-03-11T08:10:00.000Z',
    lastSuccessfulSyncAt: '2026-03-11T07:48:00.000Z',
    lastActivityAt: '2026-03-11T08:16:00.000Z',
    lastError: null,
    primaryWindow: {
      usedPercent: 0,
      usedText: '0 requests',
      limitText: '120 requests',
      resetsAt: '2026-03-11T13:00:00.000Z',
      windowDurationMins: 300,
      actualUsage: usage(0, 0, 0, 0, 0, 0),
    },
    secondaryWindow: {
      usedPercent: 0,
      usedText: '0 requests',
      limitText: '500 requests',
      resetsAt: '2026-03-18T00:00:00.000Z',
      windowDurationMins: 10080,
      actualUsage: usage(0, 0, 0, 0, 0, 0),
    },
    credits: {
      hasCredits: false,
      unlimited: false,
      balance: null,
    },
    tags: [
      {
        id: 5,
        name: 'fallback',
        routingRule: defaultEffectiveRoutingRule,
      },
    ],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    localLimits: {
      primaryLimit: 120,
      secondaryLimit: 500,
      limitUnit: 'requests',
    },
  },
]

const labels = {
  selectPage: 'Select current page',
  selectRow: (name: string) => `Select ${name}`,
  account: 'Account',
  sync: 'Sync / Call',
  lastSuccess: 'Sync',
  lastCall: 'Call',
  latestAction: 'Latest',
  windows: 'Windows',
  never: 'Never',
  primary: '5h',
  primaryShort: '5h',
  secondary: '7d',
  secondaryShort: '7d',
  nextReset: 'Reset',
  unknown: 'Unknown',
  requestsMetric: 'Req',
  tokensMetric: 'Token',
  costMetric: 'Cost',
  inputTokensMetric: 'Input',
  outputTokensMetric: 'Output',
  cacheInputTokensMetric: 'Cached input',
  unavailable: 'Unavailable',
  oauth: 'OAuth',
  apiKey: 'API key',
  duplicate: 'Duplicate',
  mother: 'Mother',
  hiddenTagsA11y: (count: number, names: string) => `Show ${count} hidden tags: ${names}`,
  workStatus: (status: string) =>
    ({
      working: 'Working',
      degraded: 'Degraded',
      idle: 'Idle',
      rate_limited: 'Rate limited',
      unavailable: 'Unavailable',
    })[status] ?? status,
  workStatusCount: (count: number) => `Working ${count}`,
  enableStatus: (status: string) =>
    ({
      enabled: 'Enabled',
      disabled: 'Disabled',
    })[status] ?? status,
  healthStatus: (status: string) =>
    ({
      normal: 'Normal',
      needs_reauth: 'Needs reauth',
      upstream_unavailable: 'Upstream unavailable',
      upstream_rejected: 'Upstream rejected',
      error_other: 'Other error',
      error: 'Error',
    })[status] ?? status,
  syncState: (status: string) =>
    ({
      idle: 'Sync idle',
      syncing: 'Syncing',
    })[status] ?? status,
  action: (action?: string | null) =>
    ({
      route_hard_unavailable: 'Hard unavailable',
      route_retryable_failure: 'Temporary upstream failure',
      route_cooldown_started: 'Route cooldown',
      sync_failed: 'Sync failed',
    })[action ?? ''] ?? action ?? null,
  actionSource: (source?: string | null) =>
    ({
      call: 'Call',
      sync_maintenance: 'Maintenance sync',
    })[source ?? ''] ?? source ?? null,
  actionReason: (reason?: string | null) =>
    ({
      upstream_http_429_quota_exhausted: 'Weekly cap exhausted',
      upstream_server_overloaded: 'Upstream is temporarily overloaded',
      reauth_required: 'Needs reauth',
    })[reason ?? ''] ?? reason ?? null,
  latestActionFieldAction: 'Action',
  latestActionFieldSource: 'Source',
  latestActionFieldReason: 'Reason',
  latestActionFieldHttpStatus: 'HTTP',
  latestActionFieldOccurredAt: 'Occurred',
  latestActionFieldMessage: 'Message',
}

const chineseLabels = {
  ...labels,
  selectPage: '选择当前页',
  selectRow: (name: string) => `选择 ${name}`,
  account: '账号',
  sync: '同步 / 调用',
  lastSuccess: '最近成功同步',
  lastCall: '最近调用',
  latestAction: '最近动作',
  windows: '窗口',
  never: '从未',
  unknown: '未知',
  requestsMetric: '请求',
  tokensMetric: 'Token',
  costMetric: '金额',
  inputTokensMetric: '输入',
  outputTokensMetric: '输出',
  cacheInputTokensMetric: '缓存输入',
  unavailable: '不可用',
  oauth: 'OAuth',
  apiKey: 'API Key',
  duplicate: '重复',
  mother: '母号',
  workStatus: (status: string) =>
    ({
      working: '工作中',
      degraded: '工作降级',
      idle: '空闲',
      rate_limited: '限流中',
      unavailable: '不可用',
    })[status] ?? status,
  workStatusCount: (count: number) => `工作中 ${count}`,
  enableStatus: (status: string) =>
    ({
      enabled: '启用',
      disabled: '停用',
    })[status] ?? status,
  healthStatus: (status: string) =>
    ({
      normal: '正常',
      needs_reauth: '重新授权',
      upstream_unavailable: '上游不可用',
      upstream_rejected: '上游拒绝',
      error_other: '其他错误',
      error: '错误',
    })[status] ?? status,
  syncState: (status: string) =>
    ({
      idle: '同步空闲',
      syncing: '同步中',
    })[status] ?? status,
  action: (action?: string | null) =>
    ({
      route_hard_unavailable: '硬拒绝',
      route_retryable_failure: '临时上游失败',
      route_cooldown_started: '冷却开始',
      sync_failed: '同步失败',
    })[action ?? ''] ?? action ?? null,
  actionSource: (source?: string | null) =>
    ({
      call: '调用',
      sync_maintenance: '维护同步',
    })[source ?? ''] ?? source ?? null,
  actionReason: (reason?: string | null) =>
    ({
      upstream_http_429_quota_exhausted: '周限额耗尽',
      upstream_server_overloaded: '上游暂时过载',
      reauth_required: '需要重新授权',
    })[reason ?? ''] ?? reason ?? null,
  latestActionFieldAction: '动作',
  latestActionFieldSource: '来源',
  latestActionFieldReason: '原因',
  latestActionFieldHttpStatus: '状态码',
  latestActionFieldOccurredAt: '发生时间',
  latestActionFieldMessage: '消息',
}

const meta = {
  title: 'Account Pool/Components/Upstream Accounts Table',
  component: UpstreamAccountsTable,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
        <div className="mx-auto max-w-6xl">
          <Story />
        </div>
      </div>
    ),
  ],
  args: {
    items,
    selectedId: 11,
    selectedAccountIds: new Set<number>(),
    onSelect: () => undefined,
    onToggleSelected: () => undefined,
    onToggleSelectAllCurrentPage: () => undefined,
    emptyTitle: 'No upstream account yet',
    emptyDescription: 'Create an OAuth or API key account to start building the pool.',
    labels,
  },
} satisfies Meta<typeof UpstreamAccountsTable>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const NeedsAttentionSelected: Story = {
  args: {
    selectedId: 12,
  },
}

export const MotherBadgeContrastDark: Story = {
  args: {
    items: [items[0]],
    selectedId: 11,
    labels: chineseLabels,
  },
  globals: {
    themeMode: 'dark',
  },
  parameters: {
    docs: {
      description: {
        story: '暗色列表行中的母号 badge 回归基线，覆盖与启用/同步等同排标签并列时的可辨识度。',
      },
    },
  },
}

export const DuplicateIdentity: Story = {
  args: {
    items: [
      {
        ...items[0],
        duplicateInfo: {
          peerAccountIds: [27, 35],
          reasons: ['sharedChatgptAccountId', 'sharedChatgptUserId'],
        },
      },
      items[1],
    ],
    selectedId: 11,
  },
}

export const CompactLongLabels: Story = {
  args: {
    items: [
      {
        ...items[0],
        displayName: 'Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title',
        groupName: 'production-apac-primary-operators',
      },
      {
        ...items[1],
        duplicateInfo: {
          peerAccountIds: [11, 27],
          reasons: ['sharedChatgptUserId'],
        },
        enabled: false,
        enableStatus: 'disabled',
        workStatus: 'idle',
        healthStatus: 'normal',
        syncState: 'idle',
        status: 'disabled',
        displayStatus: 'disabled',
        planType: null,
      },
    ],
    selectedId: 12,
  },
}

export const MissingSecondaryWindow: Story = {
  args: {
    items: [
      {
        ...items[1],
        displayName: 'Team key - missing weekly limit',
        primaryWindow: {
          usedPercent: 18,
          usedText: '18 requests',
          limitText: '120 requests',
          resetsAt: '2026-03-11T13:00:00.000Z',
          windowDurationMins: 300,
          actualUsage: usage(6, 12450, 0.0812, 7320, 4330, 800),
        },
        secondaryWindow: null,
        localLimits: {
          primaryLimit: 120,
          secondaryLimit: null,
          limitUnit: 'requests',
        },
      },
    ],
    selectedId: 12,
    labels: chineseLabels,
  },
}

export const AvailabilityBadges: Story = {
  args: {
    items: [
      items[0],
      {
        ...items[0],
        id: 13,
        displayName: 'Available idle badge',
        isMother: false,
        workStatus: 'idle',
        activeConversationCount: 0,
        duplicateInfo: null,
        tags: [],
      },
      {
        ...items[1],
        id: 14,
        displayName: 'Degraded badge visible',
        workStatus: 'degraded',
        activeConversationCount: 1,
      },
      {
        ...items[1],
        id: 16,
        displayName: 'Rate-limited badge visible',
        workStatus: 'rate_limited',
        activeConversationCount: 4,
      },
      {
        ...items[0],
        id: 15,
        displayName: 'Unavailable badge hidden',
        isMother: false,
        displayStatus: 'upstream_unavailable',
        workStatus: 'unavailable',
        healthStatus: 'upstream_unavailable',
        activeConversationCount: 2,
        tags: [],
      },
    ],
    selectedId: 11,
  },
}

export const Empty: Story = {
  args: {
    items: [],
    selectedId: null,
  },
}
