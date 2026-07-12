import type { DemoScene } from './runtime'
import { publishDemoRealtime } from './events'

export type DemoAction = {
  id: number
  label: string
  at: string
}

type DemoState = {
  scene: DemoScene
  revision: number
  actions: DemoAction[]
  settings: Record<string, unknown>
  externalApiKeys: Array<Record<string, unknown>>
  accounts: Array<Record<string, unknown>>
}

type DemoListener = () => void

// Keep active demo data visually current while preserving a stable timestamp per runtime session.
const DEMO_NOW = new Date().toISOString()

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T
}

const SENSITIVE_FIELD = /api[-_]?key|authorization|cookie|credential|oauth|password|secret|session|token/i

function safePayload(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(safePayload)
  if (typeof value !== 'object' || value === null) return value
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .filter(([key]) => !SENSITIVE_FIELD.test(key))
      .map(([key, nested]) => [key, safePayload(nested)]),
  )
}

function createSettings() {
  return {
    proxy: {
      hijackEnabled: true,
      mergeUpstreamEnabled: true,
      fastModeRewriteMode: 'disabled',
      upstream429MaxRetries: 3,
      websocketEnabled: true,
      upstreamWebsocketDefaultEnabled: true,
      requestBodyLoggingEnabled: true,
      responseBodyLoggingEnabled: true,
      encryptedSessionOwnerRoutingEnabled: false,
      defaultHijackEnabled: false,
      models: ['gpt-5.6-sol', 'gpt-5.6-terra', 'gpt-5.4-mini'],
      enabledModels: ['gpt-5.6-sol', 'gpt-5.6-terra'],
    },
    forwardProxy: {
      proxyUrls: [
        'socks5://demo-tokyo.invalid:1080',
        'http://demo-frankfurt.invalid:8080',
        'socks5://demo-singapore.invalid:1080',
        'socks5://demo-sydney.invalid:1080',
        'http://demo-virginia.invalid:8080',
      ],
      subscriptionUrls: ['https://demo.invalid/subscription'],
      subscriptionUpdateIntervalSecs: 3600,
      nodes: [
        {
          key: 'demo-tokyo',
          source: 'manual',
          displayName: 'Tokyo demo relay',
          endpointUrl: 'socks5://demo-tokyo.invalid:1080',
          weight: 0.92,
          penalized: false,
          stats: {
            oneMinute: { attempts: 18, successRate: 0.97, avgLatencyMs: 184 },
            fifteenMinutes: { attempts: 254, successRate: 0.95, avgLatencyMs: 202 },
            oneHour: { attempts: 1038, successRate: 0.94, avgLatencyMs: 219 },
            oneDay: { attempts: 19680, successRate: 0.93, avgLatencyMs: 244 },
            sevenDays: { attempts: 137720, successRate: 0.94, avgLatencyMs: 238 },
          },
        },
        {
          key: 'demo-frankfurt',
          source: 'subscription',
          displayName: 'Frankfurt recovery relay',
          endpointUrl: 'http://demo-frankfurt.invalid:8080',
          weight: 0.67,
          penalized: false,
          stats: {
            oneMinute: { attempts: 7, successRate: 1, avgLatencyMs: 263 },
            fifteenMinutes: { attempts: 98, successRate: 0.98, avgLatencyMs: 278 },
            oneHour: { attempts: 411, successRate: 0.97, avgLatencyMs: 286 },
            oneDay: { attempts: 7342, successRate: 0.96, avgLatencyMs: 291 },
            sevenDays: { attempts: 51234, successRate: 0.96, avgLatencyMs: 287 },
          },
        },
        {
          key: 'demo-singapore',
          source: 'manual',
          displayName: 'Singapore warm standby',
          endpointUrl: 'socks5://demo-singapore.invalid:1080',
          weight: 0.41,
          penalized: false,
          stats: {
            oneMinute: { attempts: 3, successRate: 1, avgLatencyMs: 228 },
            fifteenMinutes: { attempts: 46, successRate: 0.98, avgLatencyMs: 236 },
            oneHour: { attempts: 184, successRate: 0.98, avgLatencyMs: 242 },
            oneDay: { attempts: 3568, successRate: 0.97, avgLatencyMs: 249 },
            sevenDays: { attempts: 24211, successRate: 0.97, avgLatencyMs: 247 },
          },
        },
        {
          key: 'demo-sydney',
          source: 'subscription',
          displayName: 'Sydney analytics relay',
          endpointUrl: 'socks5://demo-sydney.invalid:1080',
          weight: 0.54,
          penalized: false,
          stats: {
            oneMinute: { attempts: 11, successRate: 0.99, avgLatencyMs: 301 },
            fifteenMinutes: { attempts: 112, successRate: 0.98, avgLatencyMs: 315 },
            oneHour: { attempts: 546, successRate: 0.98, avgLatencyMs: 321 },
            oneDay: { attempts: 9211, successRate: 0.97, avgLatencyMs: 327 },
            sevenDays: { attempts: 63872, successRate: 0.97, avgLatencyMs: 322 },
          },
        },
        {
          key: 'demo-virginia',
          source: 'manual',
          displayName: 'Virginia batch relay',
          endpointUrl: 'http://demo-virginia.invalid:8080',
          weight: 0.36,
          penalized: false,
          stats: {
            oneMinute: { attempts: 5, successRate: 1, avgLatencyMs: 154 },
            fifteenMinutes: { attempts: 73, successRate: 0.99, avgLatencyMs: 169 },
            oneHour: { attempts: 321, successRate: 0.99, avgLatencyMs: 173 },
            oneDay: { attempts: 6154, successRate: 0.98, avgLatencyMs: 180 },
            sevenDays: { attempts: 41873, successRate: 0.98, avgLatencyMs: 176 },
          },
        },
      ],
    },
    pricing: {
      catalogVersion: 'demo-2026-07',
      entries: [
        { model: 'gpt-5.6-sol', inputPer1m: 5, outputPer1m: 30, cacheInputPer1m: 0.5, cacheReadPer1m: 0.5, cacheWritePer1m: 6.25, reasoningPer1m: null, source: 'demo' },
        { model: 'gpt-5.6-terra', inputPer1m: 2.5, outputPer1m: 15, cacheInputPer1m: 0.25, cacheReadPer1m: 0.25, cacheWritePer1m: 3.125, reasoningPer1m: null, source: 'demo' },
      ],
    },
  }
}

function createAccounts(scene: DemoScene = 'operational') {
  const attention = scene === 'attention'
  const recentAt = (minutesAgo: number) => new Date(Date.parse(DEMO_NOW) - minutesAgo * 60_000).toISOString()
  const definitions = [
    [101, 'alpha@demo.invalid', 'alpha@demo.invalid', 'production', 'team', 'oauth_codex', 'primary', 'demo-tokyo', 38, 12],
    [102, 'backup-key', null, 'standby', 'api', 'api_key_codex', 'fallback', 'demo-frankfurt', 82, null],
    [103, 'bravo@demo.invalid', 'bravo@demo.invalid', 'production', 'team', 'oauth_codex', 'primary', 'demo-tokyo', 46, 31],
    [104, 'charlie@demo.invalid', 'charlie@demo.invalid', 'production', 'plus', 'oauth_codex', 'image', 'demo-singapore', 21, 8],
    [105, 'delta@demo.invalid', 'delta@demo.invalid', 'research', 'team', 'oauth_codex', 'research', 'demo-tokyo', 63, 44],
    [106, 'echo-key', null, 'research', 'api', 'api_key_codex', 'research', 'demo-frankfurt', 19, null],
    [107, 'foxtrot@demo.invalid', 'foxtrot@demo.invalid', 'standby', 'team', 'oauth_codex', 'fallback', 'demo-singapore', 56, 29],
    [108, 'gamma-key', null, 'production', 'api', 'api_key_codex', 'primary', 'demo-tokyo', 34, null],
    [109, 'hotel@demo.invalid', 'hotel@demo.invalid', 'research', 'plus', 'oauth_codex', 'image', 'demo-singapore', 72, 51],
    [110, 'unassigned-sandbox', null, null, 'api', 'api_key_codex', 'sandbox', 'demo-frankfurt', 7, null],
    [111, 'india@demo.invalid', 'india@demo.invalid', 'edge', 'team', 'oauth_codex', 'edge', 'demo-sydney', 29, 18],
    [112, 'juliet-key', null, 'edge', 'api', 'api_key_codex', 'edge', 'demo-virginia', 48, null],
    [113, 'kilo@demo.invalid', 'kilo@demo.invalid', 'research', 'team', 'oauth_codex', 'research', 'demo-virginia', 67, 38],
    [114, 'lima@demo.invalid', 'lima@demo.invalid', 'production', 'enterprise', 'oauth_codex', 'primary', 'demo-sydney', 16, 6],
    [115, 'mike-key', null, 'standby', 'api', 'api_key_codex', 'fallback', 'demo-singapore', 54, null],
  ] as const

  return definitions.map(([id, displayName, email, groupName, planType, kind, tagName, proxyKey, primaryPercent, secondaryPercent], index) => {
    const unavailable = attention && id === 102
    const needsReauth = attention && id === 109
    const syncing = id === 107
    const status = unavailable ? 'error' : needsReauth ? 'needs_reauth' : syncing ? 'syncing' : 'active'
    const healthStatus = unavailable ? 'upstream_unavailable' : needsReauth ? 'needs_reauth' : 'normal'
    return {
      id,
      kind,
      provider: 'openai',
      displayName,
      email,
      accountId: email ? `demo-${displayName.split('@')[0]}` : null,
      chatgptAccountId: email ? `chatgpt-${id}` : null,
      groupName,
      isMother: id === 101,
      enabled: !needsReauth,
      status,
      displayStatus: unavailable ? 'upstream_unavailable' : needsReauth ? 'needs_reauth' : syncing ? 'syncing' : 'active',
      healthStatus,
      enableStatus: needsReauth ? 'disabled' : 'enabled',
      workStatus: unavailable ? 'unavailable' : id === 101 || id === 103 ? 'working' : 'idle',
      syncState: syncing ? 'syncing' : 'idle',
      planType,
      maskedApiKey: kind === 'api_key_codex' ? `sk-demo-${id.toString().slice(-2)}••••••` : null,
      hasRefreshToken: kind === 'oauth_codex',
      tags: [{ id: index % 5 + 1, name: tagName, routingRule: { allowCutIn: true, allowCutOut: true, priorityTier: tagName === 'fallback' ? 'fallback' : 'normal' } }],
      boundProxyKeys: [proxyKey],
      currentForwardProxyKey: proxyKey,
      currentForwardProxyDisplayName: proxyKey === 'demo-tokyo' ? 'Tokyo demo relay' : proxyKey === 'demo-frankfurt' ? 'Frankfurt recovery relay' : proxyKey === 'demo-sydney' ? 'Sydney analytics relay' : proxyKey === 'demo-virginia' ? 'Virginia batch relay' : 'Singapore warm standby',
      currentForwardProxyState: 'assigned',
      lastSyncedAt: recentAt(2 + index),
      lastSuccessfulSyncAt: recentAt(3 + index),
      lastActivityAt: recentAt(Math.min(40, index * 2 + 1)),
      activeConversationCount: id === 101 ? 3 : id === 103 ? 2 : 0,
      lastError: unavailable ? 'Simulated upstream timeout from recovery relay.' : needsReauth ? 'Simulated refresh token requires reauthorization.' : null,
      lastErrorAt: unavailable || needsReauth ? recentAt(6) : null,
      lastAction: unavailable ? 'mark_unavailable' : needsReauth ? 'require_reauth' : 'sync_succeeded',
      lastActionSource: 'demo_scheduler',
      lastActionReasonCode: unavailable ? 'transport_failure' : needsReauth ? 'reauth_required' : null,
      lastActionReasonMessage: unavailable ? 'The upstream did not respond before the demo timeout.' : needsReauth ? 'The upstream rejected the simulated refresh token.' : null,
      lastActionAt: recentAt(4 + index),
      createdAt: `2026-07-${String(index + 1).padStart(2, '0')}T02:00:00Z`,
      updatedAt: DEMO_NOW,
      primaryWindow: { usedPercent: primaryPercent, usedText: `${primaryPercent}%`, limitText: 'weekly', windowDurationMins: 10080, resetsAt: '2026-07-14T00:00:00Z' },
      secondaryWindow: secondaryPercent == null ? null : { usedPercent: secondaryPercent, usedText: `${secondaryPercent}%`, limitText: '5-hour', windowDurationMins: 300, resetsAt: '2026-07-10T12:00:00Z' },
      credits: kind === 'api_key_codex' ? { hasCredits: true, unlimited: false, balance: `$${(71.5 - index * 4.2).toFixed(2)}` } : { hasCredits: false, unlimited: true, balance: null },
      localLimits: { primaryLimit: 4, secondaryLimit: 2, limitUnit: 'concurrent requests' },
      compactSupport: { status: 'supported', observedAt: recentAt(45), reason: null },
      imageToolCapability: tagName === 'image' ? 'supported' : 'unknown',
      duplicateInfo: id === 103 ? { peerAccountIds: [101], reasons: ['sharedChatgptUserId'] } : null,
      effectiveRoutingRule: {
        allowCutOut: true,
        allowCutIn: tagName !== 'sandbox',
        priorityTier: tagName === 'fallback' ? 'fallback' : 'normal',
        fastModeRewriteMode: 'keep_original',
        imageToolRewriteMode: 'keep_original',
        concurrencyLimit: tagName === 'fallback' ? 2 : 4,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
        availableModels: tagName === 'image' ? ['gpt-5.6-sol', 'gpt-5.4-mini'] : ['gpt-5.6-sol', 'gpt-5.6-terra'],
        availableModelsDefined: true,
        systemDeniedModels: [],
        sourceTagIds: [index % 5 + 1],
        sourceTagNames: [tagName],
      },
    }
  })
}

function createState(scene: DemoScene): DemoState {
  return {
    scene,
    revision: 0,
    actions: [],
    settings: createSettings(),
    externalApiKeys: [
      { id: 41, name: 'Production dashboard', status: 'active', prefix: 'cvm_prod', lastUsedAt: '2026-07-10T09:24:00Z', createdAt: '2026-06-19T02:00:00Z', updatedAt: DEMO_NOW },
      { id: 42, name: 'CI smoke checks', status: 'active', prefix: 'cvm_ci', lastUsedAt: '2026-07-10T08:42:00Z', createdAt: '2026-06-28T02:00:00Z', updatedAt: '2026-07-09T12:00:00Z' },
      { id: 43, name: 'Research notebook', status: 'active', prefix: 'cvm_research', lastUsedAt: '2026-07-09T17:12:00Z', createdAt: '2026-07-02T02:00:00Z', updatedAt: '2026-07-02T02:00:00Z' },
      { id: 44, name: 'Retired migration key', status: 'disabled', prefix: 'cvm_retired', lastUsedAt: '2026-06-30T09:00:00Z', createdAt: '2026-05-17T02:00:00Z', updatedAt: '2026-07-01T00:00:00Z' },
      { id: 45, name: 'Alert automation', status: 'active', prefix: 'cvm_alert', lastUsedAt: DEMO_NOW, createdAt: '2026-07-08T02:00:00Z', updatedAt: DEMO_NOW },
      { id: 46, name: 'Mobile QA console', status: 'active', prefix: 'cvm_mobile', lastUsedAt: '2026-07-10T09:18:00Z', createdAt: '2026-07-09T02:00:00Z', updatedAt: DEMO_NOW },
    ],
    accounts: createAccounts(scene),
  }
}

class DemoModel {
  #state = createState('operational')
  #listeners = new Set<DemoListener>()

  get snapshot(): DemoState {
    return this.#state
  }

  subscribe(listener: DemoListener): () => void {
    this.#listeners.add(listener)
    return () => this.#listeners.delete(listener)
  }

  setScene(scene: DemoScene) {
    if (this.#state.scene === scene) return
    this.#state = createState(scene)
    this.#emit()
  }

  reset() {
    this.#state = createState(this.#state.scene)
    this.#emit()
  }

  record(label: string) {
    this.#state = {
      ...this.#state,
      revision: this.#state.revision + 1,
      actions: [{ id: this.#state.revision + 1, label, at: DEMO_NOW }, ...this.#state.actions].slice(0, 6),
    }
    this.#emit()
  }

  updateSettings(pathname: string, payload: unknown) {
    const nextPayload = safePayload(payload)
    const update = typeof nextPayload === 'object' && nextPayload !== null
      ? nextPayload as Record<string, unknown>
      : {}
    const settings = clone(this.#state.settings)

    if (pathname === '/api/settings/proxy') {
      settings.proxy = { ...(settings.proxy as Record<string, unknown>), ...update }
    } else if (pathname === '/api/settings/forward-proxy') {
      settings.forwardProxy = { ...(settings.forwardProxy as Record<string, unknown>), ...update }
    } else if (pathname === '/api/settings/pricing') {
      settings.pricing = update
    } else {
      Object.assign(settings, update)
    }

    this.#state = {
      ...this.#state,
      settings,
    }
    this.record('模拟保存配置')
    if (pathname === '/api/settings/proxy') return clone(settings.proxy)
    if (pathname === '/api/settings/forward-proxy') return clone(settings.forwardProxy)
    if (pathname === '/api/settings/pricing') return clone(settings.pricing)
    return clone(settings)
  }

  createAccount() {
    const account = {
      ...createAccounts(this.#state.scene)[0],
      id: 1000 + this.#state.accounts.length,
      displayName: `demo-account-${this.#state.accounts.length + 1}`,
      email: null,
      accountId: null,
      status: 'active',
      groupName: 'production',
      updatedAt: DEMO_NOW,
    }
    this.#state = { ...this.#state, accounts: [account, ...this.#state.accounts] }
    this.record('模拟创建账号')
    return clone(account)
  }

  createExternalApiKey() {
    const key = {
      id: 40 + this.#state.externalApiKeys.length + 1,
      name: `Demo integration ${this.#state.externalApiKeys.length + 1}`,
      status: 'active',
      prefix: `cvm_demo_${this.#state.externalApiKeys.length + 1}`,
      lastUsedAt: null,
      createdAt: DEMO_NOW,
      updatedAt: DEMO_NOW,
    }
    this.#state = {
      ...this.#state,
      externalApiKeys: [key, ...this.#state.externalApiKeys],
    }
    this.record('模拟创建外部 API Key')
    return {
      key: clone(key),
      secret: 'demo-generated-key-not-valid',
    }
  }

  injectLiveEvent() {
    const record = {
      ...createAccounts(this.#state.scene)[0],
      id: 9911,
      invokeId: 'demo-live-event-9911',
      occurredAt: DEMO_NOW,
      createdAt: DEMO_NOW,
      source: 'proxy',
      proxyDisplayName: 'Tokyo demo relay',
      endpoint: '/v1/responses',
      model: 'gpt-5.6-sol',
      status: 'success',
      requestedServiceTier: 'priority',
      serviceTier: 'priority',
      inputTokens: 2300,
      outputTokens: 144,
      cacheInputTokens: 1980,
      totalTokens: 2444,
      cost: 0.0062,
      tUpstreamTtfbMs: 144,
      tTotalMs: 1088,
    }
    this.record('注入模拟实时事件')
    publishDemoRealtime({ type: 'records', records: [record] })
  }

  #emit() {
    this.#listeners.forEach((listener) => listener())
  }
}

export const demoModel = new DemoModel()

export function demoNow() {
  return DEMO_NOW
}
