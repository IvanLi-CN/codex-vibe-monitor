import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Icon } from '@iconify/react'
import { Badge } from '../components/ui/badge'
import { Alert } from '../components/ui/alert'
import { Button } from '../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card'
import { Input } from '../components/ui/input'
import { Switch } from '../components/ui/switch'
import { useSettings } from '../hooks/useSettings'
import {
  type ForwardProxyNodeStats,
  validateForwardProxyCandidate,
  type ForwardProxySettings,
  type PricingEntry,
  type PricingSettings,
} from '../lib/api'
import { cn } from '../lib/utils'
import { useTranslation } from '../i18n'

type PricingDraftEntry = {
  model: string
  inputPer1m: string
  outputPer1m: string
  cacheInputPer1m: string
  reasoningPer1m: string
  source: string
}

type PricingDraft = {
  catalogVersion: string
  entries: PricingDraftEntry[]
}

type ForwardProxyValidationState =
  | { status: 'idle' }
  | { status: 'validating'; message?: string }
  | { status: 'failed'; message: string }
  | { status: 'passed'; message: string; normalizedValues: string[]; discoveredNodes?: number; latencyMs?: number }

type ForwardProxyModalKind = 'proxyBatch' | 'subscriptionUrl'

type ForwardProxyBatchValidationStatus = 'validating' | 'available' | 'unavailable'

type ForwardProxyBatchRoundResult = {
  round: number
  ok: boolean
  timedOut: boolean
  latencyMs?: number
  message: string
}

type ForwardProxyBatchValidationItem = {
  key: string
  order: number
  rawValue: string
  normalizedValue: string
  displayName: string
  protocolName: string
  status: ForwardProxyBatchValidationStatus
  latencyMs?: number
  message: string
  rounds: ForwardProxyBatchRoundResult[]
  completedRounds: number
  totalRounds: number
  successRounds: number
  lastRound?: ForwardProxyBatchRoundResult
}

type ForwardProxyTableNode = {
  key: string
  displayName: string
  endpointUrl?: string
  weight?: number
  penalized: boolean
  stats: ForwardProxyNodeStats
}

const AUTO_SAVE_DEBOUNCE_MS = 600
const FORWARD_PROXY_BATCH_VALIDATION_CONCURRENCY = 5
const FORWARD_PROXY_BATCH_VALIDATION_ROUNDS = 12
const pricingTableHeaderCellClass =
  'px-4 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/65'
const pricingTableBodyCellClass = 'align-middle px-4 py-3'

function normalizeDraftEntry(entry: PricingDraftEntry): PricingDraftEntry {
  return {
    ...entry,
    model: entry.model.trim(),
    source: entry.source.trim() || 'custom',
  }
}

function toPricingDraft(pricing: PricingSettings): PricingDraft {
  return {
    catalogVersion: pricing.catalogVersion,
    entries: pricing.entries.map((entry) => ({
      model: entry.model,
      inputPer1m: String(entry.inputPer1m),
      outputPer1m: String(entry.outputPer1m),
      cacheInputPer1m: entry.cacheInputPer1m == null ? '' : String(entry.cacheInputPer1m),
      reasoningPer1m: entry.reasoningPer1m == null ? '' : String(entry.reasoningPer1m),
      source: entry.source,
    })),
  }
}

function parsePricingValue(raw: string, allowEmpty: boolean): number | null | undefined {
  const text = raw.trim()
  if (!text) {
    return allowEmpty ? null : undefined
  }
  const parsed = Number(text)
  if (!Number.isFinite(parsed)) return undefined
  return parsed
}

function parsePricingDraft(draft: PricingDraft): { value?: PricingSettings; error?: string } {
  const catalogVersion = draft.catalogVersion.trim()
  if (!catalogVersion) {
    return { error: 'settings.pricing.errors.catalogVersionRequired' }
  }

  const entries: PricingEntry[] = []
  const seen = new Set<string>()
  for (const rawEntry of draft.entries.map(normalizeDraftEntry)) {
    const model = rawEntry.model
    if (!model) {
      return { error: 'settings.pricing.errors.modelRequired' }
    }
    if (model.length > 128) {
      return { error: 'settings.pricing.errors.modelTooLong' }
    }
    if (seen.has(model)) {
      return { error: 'settings.pricing.errors.modelDuplicated' }
    }
    seen.add(model)

    const inputPer1m = parsePricingValue(rawEntry.inputPer1m, false)
    const outputPer1m = parsePricingValue(rawEntry.outputPer1m, false)
    const cacheInputPer1m = parsePricingValue(rawEntry.cacheInputPer1m, true)
    const reasoningPer1m = parsePricingValue(rawEntry.reasoningPer1m, true)

    if (inputPer1m == null || outputPer1m == null) {
      return { error: 'settings.pricing.errors.numberInvalid' }
    }
    if (cacheInputPer1m === undefined || reasoningPer1m === undefined) {
      return { error: 'settings.pricing.errors.numberInvalid' }
    }
    if (
      inputPer1m < 0 ||
      outputPer1m < 0 ||
      (cacheInputPer1m != null && cacheInputPer1m < 0) ||
      (reasoningPer1m != null && reasoningPer1m < 0)
    ) {
      return { error: 'settings.pricing.errors.numberNegative' }
    }

    entries.push({
      model,
      inputPer1m,
      outputPer1m,
      cacheInputPer1m,
      reasoningPer1m,
      source: rawEntry.source || 'custom',
    })
  }

  entries.sort((a, b) => a.model.localeCompare(b.model))
  return {
    value: {
      catalogVersion,
      entries,
    },
  }
}

function stablePricingKey(value: PricingSettings | null): string {
  if (!value) return 'null'
  return JSON.stringify({
    catalogVersion: value.catalogVersion,
    entries: [...value.entries].sort((a, b) => a.model.localeCompare(b.model)),
  })
}

function sourceBadgeVariant(source: string): 'success' | 'warning' | 'secondary' {
  if (source === 'official') return 'success'
  if (source === 'temporary') return 'warning'
  return 'secondary'
}

function appendUniqueItem(list: string[], value: string): string[] {
  const trimmed = value.trim()
  if (!trimmed) return list
  if (list.includes(trimmed)) return list
  return [...list, trimmed]
}

function appendUniqueItems(list: string[], values: string[]): string[] {
  let next = list
  for (const value of values) {
    next = appendUniqueItem(next, value)
  }
  return next
}

function parseMultilineItems(raw: string): string[] {
  const seen = new Set<string>()
  const items: string[] = []
  for (const line of raw.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed || seen.has(trimmed)) continue
    seen.add(trimmed)
    items.push(trimmed)
  }
  return items
}

function decodeBase64Maybe(raw: string): string | null {
  const compact = raw.trim().replace(/\s+/g, '')
  if (!compact) return null
  const normalized = compact.replace(/-/g, '+').replace(/_/g, '/')
  const padded = normalized + '='.repeat((4 - (normalized.length % 4)) % 4)
  try {
    return atob(padded)
  } catch {
    return null
  }
}

function labelFromUrl(url: URL): string | null {
  const fragment = decodeURIComponent(url.hash.replace(/^#/, '')).trim()
  if (fragment) return fragment

  const host = url.hostname || url.host
  if (!host) return null
  const defaultPort = url.protocol === 'https:' ? '443' : url.protocol === 'http:' ? '80' : ''
  const port = url.port || defaultPort
  return port ? `${host}:${port}` : host
}

function extractProxyDisplayName(raw: string): string | null {
  const candidate = raw.trim()
  if (!candidate) return null

  if (candidate.startsWith('vmess://')) {
    const payload = candidate.slice('vmess://'.length).split('#')[0].split('?')[0]
    const decoded = decodeBase64Maybe(payload)
    if (!decoded) return null
    try {
      const parsed = JSON.parse(decoded) as { ps?: string; add?: string; port?: string | number }
      const display = (parsed.ps || '').trim()
      if (display) return display
      if (parsed.add) return parsed.port ? `${parsed.add}:${parsed.port}` : parsed.add
    } catch {
      return null
    }
    return null
  }

  if (candidate.startsWith('ss://')) {
    const fragment = candidate.split('#')[1]
    if (fragment) {
      const decoded = decodeURIComponent(fragment).trim()
      if (decoded) return decoded
    }
  }

  try {
    return labelFromUrl(new URL(candidate))
  } catch {
    return null
  }
}

function extractProxyProtocolName(raw: string): string | null {
  const candidate = raw.trim()
  if (!candidate) return null

  const schemeFromUrl = (() => {
    try {
      return new URL(candidate).protocol.replace(/:$/, '').toLowerCase()
    } catch {
      return null
    }
  })()
  const scheme = schemeFromUrl ?? candidate.match(/^([a-zA-Z][a-zA-Z0-9+.-]*):\/\//)?.[1]?.toLowerCase() ?? null
  if (!scheme) return null

  const schemeDisplay: Record<string, string> = {
    http: 'HTTP',
    https: 'HTTPS',
    socks: 'SOCKS',
    socks5: 'SOCKS5',
    socks5h: 'SOCKS5H',
    vmess: 'VMESS',
    vless: 'VLESS',
    trojan: 'TROJAN',
    ss: 'SS',
  }
  const label = schemeDisplay[scheme] ?? scheme.toUpperCase()
  if (label.length <= 10) return label
  return `${label.slice(0, 10)}…`
}

function sortBatchResults(items: Iterable<ForwardProxyBatchValidationItem>): ForwardProxyBatchValidationItem[] {
  return [...items].sort((a, b) => a.order - b.order)
}

function formatSuccessRate(value?: number): string {
  if (value == null || Number.isNaN(value)) return '—'
  return `${(value * 100).toFixed(1)}%`
}

function formatLatency(value?: number): string {
  if (value == null || Number.isNaN(value)) return '—'
  return `${value.toFixed(0)} ms`
}

function isForwardProxyValidationTimeout(message: string): boolean {
  const normalized = message.toLowerCase()
  return normalized.includes('timed out') || normalized.includes('timeout') || normalized.includes('超时')
}

function isForwardProxyBackendServerError(message: string): boolean {
  const normalized = message.toLowerCase()
  return (
    normalized.includes('request failed: 500') ||
    normalized.includes('request failed: 502') ||
    normalized.includes('request failed: 503') ||
    normalized.includes('request failed: 504')
  )
}

function isForwardProxyBackendUnreachable(message: string): boolean {
  const normalized = message.toLowerCase()
  return (
    normalized.includes('failed to fetch') ||
    normalized.includes('networkerror') ||
    normalized.includes('load failed') ||
    normalized.includes('network request failed')
  )
}

function emptyForwardProxyNodeStats(): ForwardProxyNodeStats {
  return {
    oneMinute: { attempts: 0 },
    fifteenMinutes: { attempts: 0 },
    oneHour: { attempts: 0 },
    oneDay: { attempts: 0 },
    sevenDays: { attempts: 0 },
  }
}

export default function SettingsPage() {
  const { t } = useTranslation()
  const {
    settings,
    isLoading,
    isProxySaving,
    isForwardProxySaving,
    isPricingSaving,
    pricingRollbackVersion,
    error,
    saveProxy,
    saveForwardProxy,
    savePricing,
  } = useSettings()

  const [pricingDraft, setPricingDraft] = useState<PricingDraft | null>(null)
  const [pricingErrorKey, setPricingErrorKey] = useState<string | null>(null)
  const [forwardProxyUrls, setForwardProxyUrls] = useState<string[]>([])
  const [forwardProxySubscriptionUrls, setForwardProxySubscriptionUrls] = useState<string[]>([])
  const [forwardProxyIntervalSecs, setForwardProxyIntervalSecs] = useState('3600')
  const [forwardProxyInsertDirect, setForwardProxyInsertDirect] = useState(true)
  const [forwardProxyDirty, setForwardProxyDirty] = useState(false)
  const [forwardProxyModalKind, setForwardProxyModalKind] = useState<ForwardProxyModalKind | null>(null)
  const [forwardProxyModalStep, setForwardProxyModalStep] = useState<1 | 2>(1)
  const [forwardProxyModalInput, setForwardProxyModalInput] = useState('')
  const [forwardProxyBatchResults, setForwardProxyBatchResults] = useState<ForwardProxyBatchValidationItem[]>([])
  const [forwardProxyBatchTooltipKey, setForwardProxyBatchTooltipKey] = useState<string | null>(null)
  const [forwardProxyValidation, setForwardProxyValidation] = useState<ForwardProxyValidationState>({ status: 'idle' })
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const forwardProxyBatchValidationRunRef = useRef(0)
  const lastSyncedPricingKeyRef = useRef<string | null>(null)
  const lastHandledRollbackVersionRef = useRef(pricingRollbackVersion)

  useEffect(() => {
    if (!settings?.pricing) return
    const nextPricing = settings.pricing
    const nextPricingKey = stablePricingKey(nextPricing)

    setPricingDraft((current) => {
      if (!current) {
        lastSyncedPricingKeyRef.current = nextPricingKey
        return toPricingDraft(nextPricing)
      }

      const parsedCurrent = parsePricingDraft(current)
      if (!parsedCurrent.value) {
        return current
      }

      if (pricingRollbackVersion !== lastHandledRollbackVersionRef.current) {
        lastHandledRollbackVersionRef.current = pricingRollbackVersion
        lastSyncedPricingKeyRef.current = nextPricingKey
        return toPricingDraft(nextPricing)
      }

      const currentDraftKey = stablePricingKey(parsedCurrent.value)
      if (currentDraftKey === nextPricingKey) {
        lastSyncedPricingKeyRef.current = nextPricingKey
        return current
      }

      if (lastSyncedPricingKeyRef.current == null || currentDraftKey === lastSyncedPricingKeyRef.current) {
        lastSyncedPricingKeyRef.current = nextPricingKey
        return toPricingDraft(nextPricing)
      }

      return current
    })
  }, [pricingRollbackVersion, settings?.pricing])

  useEffect(() => {
    if (!settings?.forwardProxy) return
    if (forwardProxyDirty && !isForwardProxySaving) return
    setForwardProxyUrls(settings.forwardProxy.proxyUrls)
    setForwardProxySubscriptionUrls(settings.forwardProxy.subscriptionUrls)
    setForwardProxyIntervalSecs(String(settings.forwardProxy.subscriptionUpdateIntervalSecs))
    setForwardProxyInsertDirect(settings.forwardProxy.insertDirect)
    setForwardProxyDirty(false)
  }, [forwardProxyDirty, isForwardProxySaving, settings?.forwardProxy])

  const currentProxy = settings?.proxy ?? null
  const currentForwardProxy = settings?.forwardProxy ?? null
  const forwardProxyTableNodes = useMemo<ForwardProxyTableNode[]>(() => {
    const persistedNodes = currentForwardProxy?.nodes ?? []
    const tableNodes: ForwardProxyTableNode[] = persistedNodes.map((node) => ({
      key: node.key,
      displayName: node.displayName,
      endpointUrl: node.endpointUrl,
      weight: node.weight,
      penalized: node.penalized,
      stats: node.stats,
    }))

    const endpointUrlSet = new Set(
      persistedNodes
        .map((node) => node.endpointUrl?.trim())
        .filter((candidate): candidate is string => Boolean(candidate)),
    )

    const hasDirectNode = persistedNodes.some((node) => node.source === 'direct' || node.key === '__direct__')
    if (forwardProxyInsertDirect && !hasDirectNode) {
      tableNodes.unshift({
        key: '__draft_direct__',
        displayName: t('settings.forwardProxy.directLabel'),
        weight: undefined,
        penalized: false,
        stats: emptyForwardProxyNodeStats(),
      })
    }

    for (const rawUrl of forwardProxyUrls) {
      const candidate = rawUrl.trim()
      if (!candidate || endpointUrlSet.has(candidate)) continue
      endpointUrlSet.add(candidate)
      tableNodes.push({
        key: `__draft__${candidate}`,
        displayName: extractProxyDisplayName(candidate) || t('settings.forwardProxy.modal.unknownNode'),
        endpointUrl: candidate,
        weight: undefined,
        penalized: false,
        stats: emptyForwardProxyNodeStats(),
      })
    }

    return tableNodes
  }, [currentForwardProxy?.nodes, forwardProxyInsertDirect, forwardProxyUrls, t])
  const enabledPresetModelSet = useMemo(
    () => new Set(currentProxy?.enabledModels ?? []),
    [currentProxy?.enabledModels],
  )
  const forwardProxyIntervalOptions = useMemo(
    () => [
      { value: '60', label: t('settings.forwardProxy.interval.1m') },
      { value: '300', label: t('settings.forwardProxy.interval.5m') },
      { value: '900', label: t('settings.forwardProxy.interval.15m') },
      { value: '3600', label: t('settings.forwardProxy.interval.1h') },
      { value: '21600', label: t('settings.forwardProxy.interval.6h') },
      { value: '86400', label: t('settings.forwardProxy.interval.1d') },
    ],
    [t],
  )

  const remotePricingKey = useMemo(() => stablePricingKey(settings?.pricing ?? null), [settings?.pricing])
  const remotePricingKeyRef = useRef(remotePricingKey)

  useEffect(() => {
    remotePricingKeyRef.current = remotePricingKey
  }, [remotePricingKey])

  const triggerPricingSave = useCallback(
    (forceImmediate: boolean) => {
      if (!pricingDraft) return
      const parsed = parsePricingDraft(pricingDraft)
      if (!parsed.value) {
        if (debounceTimerRef.current) {
          clearTimeout(debounceTimerRef.current)
          debounceTimerRef.current = null
        }
        setPricingErrorKey(parsed.error ?? 'settings.pricing.errors.numberInvalid')
        return
      }
      const draftKey = stablePricingKey(parsed.value)
      if (draftKey === remotePricingKeyRef.current) {
        if (debounceTimerRef.current) {
          clearTimeout(debounceTimerRef.current)
          debounceTimerRef.current = null
        }
        setPricingErrorKey(null)
        return
      }

      setPricingErrorKey(null)
      const runSave = () => {
        void savePricing(parsed.value as PricingSettings)
      }

      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current)
      }

      if (forceImmediate) {
        runSave()
      } else {
        debounceTimerRef.current = setTimeout(runSave, AUTO_SAVE_DEBOUNCE_MS)
      }
    },
    [pricingDraft, savePricing],
  )

  useEffect(() => {
    if (!pricingDraft) return
    triggerPricingSave(false)
    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current)
      }
    }
  }, [pricingDraft, triggerPricingSave])

  const handleToggleHijack = useCallback(() => {
    if (!currentProxy) return
    void saveProxy({
      ...currentProxy,
      hijackEnabled: !currentProxy.hijackEnabled,
      mergeUpstreamEnabled: currentProxy.mergeUpstreamEnabled,
      enabledModels: currentProxy.enabledModels,
    })
  }, [currentProxy, saveProxy])

  const handleToggleMergeUpstream = useCallback(() => {
    if (!currentProxy || !currentProxy.hijackEnabled) return
    void saveProxy({
      ...currentProxy,
      mergeUpstreamEnabled: !currentProxy.mergeUpstreamEnabled,
      enabledModels: currentProxy.enabledModels,
    })
  }, [currentProxy, saveProxy])

  const handleTogglePresetModel = useCallback(
    (modelId: string) => {
      if (!currentProxy) return
      const enabled = new Set(currentProxy.enabledModels)
      if (enabled.has(modelId)) {
        enabled.delete(modelId)
      } else {
        enabled.add(modelId)
      }
      void saveProxy({
        ...currentProxy,
        enabledModels: currentProxy.models.filter((candidate) => enabled.has(candidate)),
      })
    },
    [currentProxy, saveProxy],
  )

  const handlePricingFieldChange = useCallback(
    (index: number, field: keyof PricingDraftEntry, value: string) => {
      setPricingDraft((current) => {
        if (!current) return current
        const nextEntries = [...current.entries]
        nextEntries[index] = {
          ...nextEntries[index],
          [field]: value,
        }
        return {
          ...current,
          entries: nextEntries,
        }
      })
    },
    [],
  )

  const handleCatalogVersionChange = useCallback((value: string) => {
    setPricingDraft((current) => {
      if (!current) return current
      return {
        ...current,
        catalogVersion: value,
      }
    })
  }, [])

  const handleAddPricingEntry = useCallback(() => {
    setPricingDraft((current) => {
      if (!current) return current
      return {
        ...current,
        entries: [
          ...current.entries,
          {
            model: '',
            inputPer1m: '0',
            outputPer1m: '0',
            cacheInputPer1m: '',
            reasoningPer1m: '',
            source: 'custom',
          },
        ],
      }
    })
  }, [])

  const handleRemovePricingEntry = useCallback((index: number) => {
    setPricingDraft((current) => {
      if (!current) return current
      return {
        ...current,
        entries: current.entries.filter((_, candidateIndex) => candidateIndex !== index),
      }
    })
  }, [])

  const handleForwardProxySave = useCallback(() => {
    if (!currentForwardProxy) return
    const parsedInterval = Number(forwardProxyIntervalSecs)
    const intervalSecs = Number.isFinite(parsedInterval) ? Math.max(60, Math.floor(parsedInterval)) : 3600
    const nextForwardProxy: ForwardProxySettings = {
      ...currentForwardProxy,
      proxyUrls: forwardProxyUrls,
      subscriptionUrls: forwardProxySubscriptionUrls,
      subscriptionUpdateIntervalSecs: intervalSecs,
      insertDirect: forwardProxyInsertDirect,
    }
    void saveForwardProxy(nextForwardProxy)
    setForwardProxyDirty(false)
  }, [
    currentForwardProxy,
    forwardProxyInsertDirect,
    forwardProxyIntervalSecs,
    forwardProxySubscriptionUrls,
    forwardProxyUrls,
    saveForwardProxy,
  ])

  const persistForwardProxyDraft = useCallback(
    (overrides?: { proxyUrls?: string[]; subscriptionUrls?: string[] }) => {
      if (!currentForwardProxy) return
      const parsedInterval = Number(forwardProxyIntervalSecs)
      const intervalSecs = Number.isFinite(parsedInterval) ? Math.max(60, Math.floor(parsedInterval)) : 3600
      const nextForwardProxy: ForwardProxySettings = {
        ...currentForwardProxy,
        proxyUrls: overrides?.proxyUrls ?? forwardProxyUrls,
        subscriptionUrls: overrides?.subscriptionUrls ?? forwardProxySubscriptionUrls,
        subscriptionUpdateIntervalSecs: intervalSecs,
        insertDirect: forwardProxyInsertDirect,
      }
      void saveForwardProxy(nextForwardProxy)
      setForwardProxyDirty(false)
    },
    [
      currentForwardProxy,
      forwardProxyInsertDirect,
      forwardProxyIntervalSecs,
      forwardProxySubscriptionUrls,
      forwardProxyUrls,
      saveForwardProxy,
    ],
  )

  const handleRemoveForwardProxyUrl = useCallback(
    (targetUrl: string) => {
      const nextProxyUrls = forwardProxyUrls.filter((item) => item !== targetUrl)
      setForwardProxyUrls(nextProxyUrls)
      persistForwardProxyDraft({ proxyUrls: nextProxyUrls })
    },
    [forwardProxyUrls, persistForwardProxyDraft],
  )

  const handleRemoveForwardProxySubscriptionUrl = useCallback(
    (targetUrl: string) => {
      const nextSubscriptionUrls = forwardProxySubscriptionUrls.filter((item) => item !== targetUrl)
      setForwardProxySubscriptionUrls(nextSubscriptionUrls)
      persistForwardProxyDraft({ subscriptionUrls: nextSubscriptionUrls })
    },
    [forwardProxySubscriptionUrls, persistForwardProxyDraft],
  )

  const openForwardProxyAddModal = useCallback((kind: ForwardProxyModalKind) => {
    forwardProxyBatchValidationRunRef.current += 1
    setForwardProxyModalKind(kind)
    setForwardProxyModalStep(1)
    setForwardProxyModalInput('')
    setForwardProxyBatchResults([])
    setForwardProxyBatchTooltipKey(null)
    setForwardProxyValidation({ status: 'idle' })
  }, [])

  const closeForwardProxyAddModal = useCallback(() => {
    forwardProxyBatchValidationRunRef.current += 1
    setForwardProxyModalKind(null)
    setForwardProxyModalStep(1)
    setForwardProxyModalInput('')
    setForwardProxyBatchResults([])
    setForwardProxyBatchTooltipKey(null)
    setForwardProxyValidation({ status: 'idle' })
  }, [])

  const resolveForwardProxyValidationErrorMessage = useCallback(
    (error: unknown) => {
      const rawMessage = error instanceof Error ? error.message : String(error ?? '')
      if (isForwardProxyValidationTimeout(rawMessage)) {
        return rawMessage
      }
      if (isForwardProxyBackendUnreachable(rawMessage)) {
        return t('settings.forwardProxy.modal.backendUnreachable')
      }
      if (isForwardProxyBackendServerError(rawMessage)) {
        return t('settings.forwardProxy.modal.backendServerError')
      }
      const trimmed = rawMessage.trim()
      return trimmed || t('settings.forwardProxy.modal.validateFailed')
    },
    [t],
  )

  const syncForwardProxyBatchValidationState = useCallback(
    (results: ForwardProxyBatchValidationItem[]) => {
      const availableCount = results.filter((item) => item.status === 'available').length
      const unavailableCount = results.filter((item) => item.status === 'unavailable').length
      const validatingCount = results.length - availableCount - unavailableCount
      if (validatingCount > 0) {
        setForwardProxyValidation({
          status: 'validating',
          message: t('settings.forwardProxy.modal.batchValidateProgress', {
            available: availableCount,
            unavailable: unavailableCount,
            validating: validatingCount,
          }),
        })
        return
      }
      setForwardProxyValidation({
        status: 'passed',
        message: t('settings.forwardProxy.modal.batchValidateSummary', {
          available: availableCount,
          unavailable: unavailableCount,
        }),
        normalizedValues: results
          .filter((item) => item.status === 'available')
          .map((item) => item.normalizedValue),
        discoveredNodes: availableCount,
      })
    },
    [t],
  )

  const runForwardProxyBatchValidationRoundTask = useCallback(
    async (params: { validationRunId: number; nodeKey: string; rawValue: string; round: number }) => {
      const { validationRunId, nodeKey, rawValue, round } = params
      const unknownNodeName = t('settings.forwardProxy.modal.unknownNode')
      const unknownProtocolName = t('settings.forwardProxy.modal.unknownProtocol')
      if (forwardProxyBatchValidationRunRef.current !== validationRunId) {
        return
      }

      let roundResult: ForwardProxyBatchRoundResult
      let normalizedValueFromResult: string | null = null
      try {
        const result = await validateForwardProxyCandidate({
          kind: 'proxyUrl',
          value: rawValue,
        })
        const message =
          result.message || (result.ok ? t('settings.forwardProxy.modal.validateSuccess') : t('settings.forwardProxy.modal.validateFailed'))
        const normalizedCandidate = result.normalizedValue?.trim()
        if (normalizedCandidate) {
          normalizedValueFromResult = normalizedCandidate
        }
        roundResult = {
          round,
          ok: result.ok,
          timedOut: !result.ok && isForwardProxyValidationTimeout(message),
          latencyMs: result.latencyMs,
          message,
        }
      } catch (err) {
        const message = resolveForwardProxyValidationErrorMessage(err)
        roundResult = {
          round,
          ok: false,
          timedOut: isForwardProxyValidationTimeout(message),
          message: message || t('settings.forwardProxy.modal.validateFailed'),
        }
      }

      if (forwardProxyBatchValidationRunRef.current !== validationRunId) {
        return
      }

      setForwardProxyBatchResults((current) => {
        const next = current.map((item) => {
          if (item.key !== nodeKey) return item
          const normalizedValue = normalizedValueFromResult ?? item.normalizedValue

          const rounds = [...item.rounds.filter((entry) => entry.round !== round), roundResult].sort(
            (lhs, rhs) => lhs.round - rhs.round,
          )
          const completedRounds = rounds.length
          const successRounds = rounds.filter((entry) => entry.ok).length
          const isCompleted = completedRounds >= FORWARD_PROXY_BATCH_VALIDATION_ROUNDS
          const nextStatus: ForwardProxyBatchValidationStatus =
            successRounds > 0 ? 'available' : isCompleted ? 'unavailable' : 'validating'

          const normalizedDisplayName = extractProxyDisplayName(normalizedValue)
          const normalizedProtocolName = extractProxyProtocolName(normalizedValue)

          return {
            ...item,
            normalizedValue,
            displayName: normalizedDisplayName || item.displayName || unknownNodeName,
            protocolName: normalizedProtocolName || item.protocolName || unknownProtocolName,
            status: nextStatus,
            latencyMs: roundResult.ok ? roundResult.latencyMs : undefined,
            message: roundResult.message,
            rounds,
            completedRounds,
            totalRounds: FORWARD_PROXY_BATCH_VALIDATION_ROUNDS,
            successRounds,
            lastRound: rounds[rounds.length - 1],
          }
        })
        syncForwardProxyBatchValidationState(next)
        return next
      })
    },
    [resolveForwardProxyValidationErrorMessage, syncForwardProxyBatchValidationState, t],
  )

  const handleValidateForwardProxyCandidate = useCallback(async () => {
    if (!forwardProxyModalKind) return
    const candidate = forwardProxyModalInput.trim()
    if (!candidate) {
      setForwardProxyValidation({
        status: 'failed',
        message: t('settings.forwardProxy.modal.required'),
      })
      return
    }

    setForwardProxyValidation({ status: 'validating' })
    if (forwardProxyModalKind === 'subscriptionUrl') {
      try {
        const result = await validateForwardProxyCandidate({
          kind: 'subscriptionUrl',
          value: candidate,
        })
        if (!result.ok) {
          setForwardProxyValidation({
            status: 'failed',
            message: result.message || t('settings.forwardProxy.modal.validateFailed'),
          })
          return
        }
        setForwardProxyValidation({
          status: 'passed',
          message: result.message || t('settings.forwardProxy.modal.validateSuccess'),
          normalizedValues: [result.normalizedValue?.trim() || candidate],
          discoveredNodes: result.discoveredNodes,
          latencyMs: result.latencyMs,
        })
      } catch (err) {
        setForwardProxyValidation({
          status: 'failed',
          message: resolveForwardProxyValidationErrorMessage(err),
        })
      }
      return
    }

    const lines = parseMultilineItems(forwardProxyModalInput)
    if (lines.length === 0) {
      setForwardProxyValidation({
        status: 'failed',
        message: t('settings.forwardProxy.modal.required'),
      })
      return
    }
    const unknownNodeName = t('settings.forwardProxy.modal.unknownNode')
    const unknownProtocolName = t('settings.forwardProxy.modal.unknownProtocol')
    setForwardProxyModalStep(2)
    const validationRunId = forwardProxyBatchValidationRunRef.current + 1
    forwardProxyBatchValidationRunRef.current = validationRunId
    setForwardProxyBatchTooltipKey(null)

    const initialResults = lines.map((rawLine, index) => ({
      key: `__batch__${index}`,
      order: index,
      rawValue: rawLine,
      normalizedValue: rawLine,
      displayName: extractProxyDisplayName(rawLine) || unknownNodeName,
      protocolName: extractProxyProtocolName(rawLine) || unknownProtocolName,
      status: 'validating' as const,
      message: t('settings.forwardProxy.modal.rowValidating'),
      rounds: [],
      completedRounds: 0,
      totalRounds: FORWARD_PROXY_BATCH_VALIDATION_ROUNDS,
      successRounds: 0,
    }))

    const sortedInitialResults = sortBatchResults(initialResults)
    setForwardProxyBatchResults(sortedInitialResults)
    syncForwardProxyBatchValidationState(sortedInitialResults)

    const workerCount = Math.min(FORWARD_PROXY_BATCH_VALIDATION_CONCURRENCY, sortedInitialResults.length)
    for (let round = 1; round <= FORWARD_PROXY_BATCH_VALIDATION_ROUNDS; round += 1) {
      if (forwardProxyBatchValidationRunRef.current !== validationRunId) return
      let nextNodeIndex = 0
      await Promise.all(
        Array.from({ length: workerCount }, async () => {
          while (true) {
            if (forwardProxyBatchValidationRunRef.current !== validationRunId) return
            const currentNodeIndex = nextNodeIndex
            nextNodeIndex += 1
            if (currentNodeIndex >= sortedInitialResults.length) return
            const currentItem = sortedInitialResults[currentNodeIndex]
            await runForwardProxyBatchValidationRoundTask({
              validationRunId,
              nodeKey: currentItem.key,
              rawValue: currentItem.rawValue,
              round,
            })
          }
        }),
      )
    }
  }, [
    forwardProxyModalInput,
    forwardProxyModalKind,
    resolveForwardProxyValidationErrorMessage,
    runForwardProxyBatchValidationRoundTask,
    syncForwardProxyBatchValidationState,
    t,
  ])

  const handleConfirmAddForwardProxyCandidate = useCallback(() => {
    if (forwardProxyValidation.status !== 'passed' || !forwardProxyModalKind) return
    if (forwardProxyModalKind !== 'subscriptionUrl') return
    if (forwardProxyValidation.normalizedValues.length === 0) return
    const nextSubscriptionUrls = appendUniqueItem(forwardProxySubscriptionUrls, forwardProxyValidation.normalizedValues[0])
    setForwardProxySubscriptionUrls(nextSubscriptionUrls)
    persistForwardProxyDraft({ subscriptionUrls: nextSubscriptionUrls })
    closeForwardProxyAddModal()
  }, [
    closeForwardProxyAddModal,
    forwardProxyModalKind,
    forwardProxySubscriptionUrls,
    forwardProxyValidation,
    persistForwardProxyDraft,
  ])

  const handleRetryBatchNode = useCallback(
    async (nodeKey: string) => {
      if (forwardProxyModalKind !== 'proxyBatch') return
      const target = forwardProxyBatchResults.find((item) => item.key === nodeKey)
      if (!target) return
      const validationRunId = forwardProxyBatchValidationRunRef.current + 1
      forwardProxyBatchValidationRunRef.current = validationRunId
      setForwardProxyBatchTooltipKey((current) => (current === nodeKey ? null : current))

      setForwardProxyBatchResults((current) => {
        const next = current.map((item) =>
          item.key === nodeKey
            ? {
                ...item,
                status: 'validating' as const,
                latencyMs: undefined,
                message: t('settings.forwardProxy.modal.rowValidating'),
                rounds: [] as ForwardProxyBatchRoundResult[],
                completedRounds: 0,
                totalRounds: FORWARD_PROXY_BATCH_VALIDATION_ROUNDS,
                successRounds: 0,
                lastRound: undefined,
              }
            : item,
        )
        syncForwardProxyBatchValidationState(next)
        return next
      })

      for (let round = 1; round <= FORWARD_PROXY_BATCH_VALIDATION_ROUNDS; round += 1) {
        if (forwardProxyBatchValidationRunRef.current !== validationRunId) return
        await runForwardProxyBatchValidationRoundTask({
          validationRunId,
          nodeKey,
          rawValue: target.rawValue,
          round,
        })
      }
    },
    [
      forwardProxyBatchResults,
      forwardProxyModalKind,
      runForwardProxyBatchValidationRoundTask,
      syncForwardProxyBatchValidationState,
      t,
    ],
  )

  const handleAddValidatedBatchNode = useCallback(
    (nodeKey: string) => {
      if (forwardProxyModalKind !== 'proxyBatch') return
      setForwardProxyBatchTooltipKey((current) => (current === nodeKey ? null : current))
      setForwardProxyBatchResults((current) => {
        const target = current.find((item) => item.key === nodeKey)
        if (!target || target.status !== 'available') return current
        const nextProxyUrls = appendUniqueItem(forwardProxyUrls, target.normalizedValue)
        setForwardProxyUrls(nextProxyUrls)
        persistForwardProxyDraft({ proxyUrls: nextProxyUrls })
        const next = current.filter((item) => item.key !== nodeKey)
        if (next.length === 0) {
          queueMicrotask(() => closeForwardProxyAddModal())
        }
        return next
      })
    },
    [closeForwardProxyAddModal, forwardProxyModalKind, forwardProxyUrls, persistForwardProxyDraft],
  )

  const handleSubmitValidatedBatchNodes = useCallback(() => {
    if (forwardProxyModalKind !== 'proxyBatch') return
    setForwardProxyBatchTooltipKey(null)
    setForwardProxyBatchResults((current) => {
      const availableItems = current.filter((item) => item.status === 'available')
      if (availableItems.length === 0) return current
      const nextProxyUrls = appendUniqueItems(
        forwardProxyUrls,
        availableItems.map((item) => item.normalizedValue),
      )
      setForwardProxyUrls(nextProxyUrls)
      persistForwardProxyDraft({ proxyUrls: nextProxyUrls })
      const next = current.filter((item) => item.status !== 'available')
      if (next.length === 0) {
        queueMicrotask(() => closeForwardProxyAddModal())
      }
      return next
    })
  }, [closeForwardProxyAddModal, forwardProxyModalKind, forwardProxyUrls, persistForwardProxyDraft])

  const forwardProxyModalTitle = forwardProxyModalKind
    ? t(
        forwardProxyModalKind === 'proxyBatch'
          ? 'settings.forwardProxy.modal.proxyBatchTitle'
          : 'settings.forwardProxy.modal.subscriptionTitle',
      )
    : ''
  const forwardProxyModalIsBatch = forwardProxyModalKind === 'proxyBatch'
  const forwardProxyModalInputLabel = forwardProxyModalKind
    ? t(
        forwardProxyModalKind === 'proxyBatch'
          ? 'settings.forwardProxy.modal.proxyBatchInputLabel'
          : 'settings.forwardProxy.modal.subscriptionInputLabel',
      )
    : ''
  const forwardProxyModalPlaceholder = forwardProxyModalKind
    ? t(
        forwardProxyModalKind === 'proxyBatch'
          ? 'settings.forwardProxy.modal.proxyBatchPlaceholder'
          : 'settings.forwardProxy.modal.subscriptionPlaceholder',
      )
    : ''
  const forwardProxyBatchAvailableCount = forwardProxyBatchResults.filter((item) => item.status === 'available').length
  const forwardProxyBatchUnavailableCount = forwardProxyBatchResults.filter((item) => item.status === 'unavailable').length
  const forwardProxyBatchValidatingCount = forwardProxyBatchResults.length - forwardProxyBatchAvailableCount - forwardProxyBatchUnavailableCount
  const forwardProxyBatchHasFirstRoundForAll =
    forwardProxyBatchResults.length > 0 && forwardProxyBatchResults.every((item) => item.completedRounds > 0)
  const forwardProxyCanConfirmAdd =
    !forwardProxyModalIsBatch &&
    forwardProxyValidation.status === 'passed' &&
    forwardProxyValidation.normalizedValues.length > 0 &&
    !isForwardProxySaving
  const forwardProxyCanSubmitBatch =
    forwardProxyModalIsBatch &&
    forwardProxyModalStep === 2 &&
    forwardProxyBatchHasFirstRoundForAll &&
    forwardProxyBatchAvailableCount > 0 &&
    !isForwardProxySaving

  if (isLoading) {
    return (
      <section className="mx-auto max-w-6xl space-y-4">
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="text-sm text-base-content/70">{t('settings.loading')}</p>
      </section>
    )
  }

  if (!settings || !currentProxy || !currentForwardProxy) {
    return (
      <section className="mx-auto max-w-6xl space-y-4">
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="text-sm text-error">{t('settings.loadError', { error: error ?? 'unknown' })}</p>
      </section>
    )
  }

  if (!pricingDraft) {
    return (
      <section className="mx-auto max-w-6xl space-y-4">
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="text-sm text-base-content/70">{t('settings.loading')}</p>
      </section>
    )
  }

  return (
    <section className="settings-page mx-auto max-w-6xl space-y-6 pb-2">
      <div>
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="mt-1 text-sm text-base-content/70">{t('settings.description')}</p>
      </div>

      <div className="grid items-start gap-6 lg:grid-cols-2">
        <Card className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
          <CardHeader className="gap-2 border-b border-base-300/70 pb-4">
            <CardTitle>{t('settings.proxy.title')}</CardTitle>
            <CardDescription>{t('settings.proxy.description')}</CardDescription>
          </CardHeader>

          <CardContent className="space-y-4 pt-4">
            <label className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4 rounded-xl border border-base-300/75 bg-base-100/72 px-4 py-4">
              <div className="space-y-1">
                <div className="font-medium leading-snug">{t('settings.proxy.hijackLabel')}</div>
                <div className="text-sm leading-snug text-base-content/70">{t('settings.proxy.hijackHint')}</div>
              </div>
              <div className="pt-0.5">
                <Switch
                  checked={currentProxy.hijackEnabled}
                  disabled={isProxySaving}
                  onCheckedChange={() => handleToggleHijack()}
                />
              </div>
            </label>

            <label className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4 rounded-xl border border-base-300/75 bg-base-100/72 px-4 py-4">
              <div className="space-y-1">
                <div className="font-medium leading-snug">{t('settings.proxy.mergeLabel')}</div>
                <div className="text-sm leading-snug text-base-content/70">{t('settings.proxy.mergeHint')}</div>
                {!currentProxy.hijackEnabled && (
                  <div className="mt-1 text-xs text-warning">{t('settings.proxy.mergeDisabledHint')}</div>
                )}
              </div>
              <div className="pt-0.5">
                <Switch
                  checked={currentProxy.mergeUpstreamEnabled}
                  disabled={isProxySaving || !currentProxy.hijackEnabled}
                  onCheckedChange={() => handleToggleMergeUpstream()}
                />
              </div>
            </label>

            <div className="rounded-xl border border-base-300/75 bg-base-200/28 p-4">
              <div className="mb-3 flex items-center justify-between gap-2">
                <div className="font-medium">{t('settings.proxy.presetModels')}</div>
                <span className="text-xs text-base-content/70">
                  {t('settings.proxy.enabledCount', {
                    count: currentProxy.enabledModels.length,
                    total: currentProxy.models.length,
                  })}
                </span>
              </div>
              <div className="space-y-2">
                {currentProxy.models.map((modelId) => {
                  const enabled = enabledPresetModelSet.has(modelId)
                  return (
                    <label
                      key={modelId}
                      className={cn(
                        'flex min-h-12 items-center gap-3 rounded-lg border px-3.5 py-2.5',
                        enabled ? 'border-primary/45 bg-primary/10' : 'border-base-300/85 bg-base-100/68',
                        isProxySaving ? 'opacity-70' : 'hover:border-primary/40',
                      )}
                    >
                      <span className="truncate pr-2 font-mono text-sm">{modelId}</span>
                      <div className="ml-auto shrink-0">
                        <Switch
                          checked={enabled}
                          disabled={isProxySaving}
                          aria-label={modelId}
                          onCheckedChange={() => handleTogglePresetModel(modelId)}
                        />
                      </div>
                    </label>
                  )
                })}
              </div>
              {currentProxy.enabledModels.length === 0 && (
                <div className="mt-2 text-xs text-warning">{t('settings.proxy.noneEnabledHint')}</div>
              )}
            </div>

            <div className="text-xs text-base-content/70">{isProxySaving ? t('settings.saving') : t('settings.autoSaved')}</div>
          </CardContent>
        </Card>

        <Card className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
          <CardHeader className="flex-row items-start justify-between gap-3 space-y-0 border-b border-base-300/70 pb-4">
            <div className="space-y-1.5">
              <CardTitle>{t('settings.pricing.title')}</CardTitle>
              <CardDescription>{t('settings.pricing.description')}</CardDescription>
            </div>
            <Button type="button" size="sm" className="h-9 gap-1.5 px-3.5" onClick={handleAddPricingEntry}>
              <Icon icon="mdi:plus" className="h-[18px] w-[18px]" aria-hidden />
              {t('settings.pricing.add')}
            </Button>
          </CardHeader>

          <CardContent className="space-y-5 pt-4">
            <div className="space-y-2">
              <label htmlFor="pricing-catalog-version" className="block text-sm font-medium text-base-content/75">
                {t('settings.pricing.catalogVersion')}
              </label>
              <Input
                id="pricing-catalog-version"
                type="text"
                className="max-w-md"
                value={pricingDraft.catalogVersion}
                onChange={(event) => handleCatalogVersionChange(event.target.value)}
                onBlur={() => triggerPricingSave(true)}
              />
            </div>

            <div className="overflow-x-auto rounded-xl border border-base-300/80 bg-base-100/72">
              <table className="w-full min-w-[56rem] table-fixed text-sm">
                <thead className="bg-base-200/70 text-[11px] uppercase tracking-[0.08em] text-base-content/65">
                  <tr>
                    <th className={cn('w-44', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.model')}</th>
                    <th className={cn('w-24', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.input')}</th>
                    <th className={cn('w-24', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.output')}</th>
                    <th className={cn('w-24', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.cacheInput')}</th>
                    <th className={cn('w-24', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.reasoning')}</th>
                    <th className={cn('w-28 whitespace-nowrap', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.source')}</th>
                    <th className={cn('w-24 whitespace-nowrap text-right', pricingTableHeaderCellClass)}>{t('settings.pricing.columns.actions')}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-base-300/65">
                  {pricingDraft.entries.map((entry, index) => (
                    <tr
                      key={index}
                      className={cn(
                        'transition-colors',
                        index % 2 === 0 ? 'bg-base-100/38' : 'bg-base-200/22',
                        'hover:bg-primary/6',
                      )}
                    >
                      <td className={pricingTableBodyCellClass}>
                        <Input
                          type="text"
                          className="h-9 px-3"
                          value={entry.model}
                          onChange={(event) => handlePricingFieldChange(index, 'model', event.target.value)}
                          onBlur={() => triggerPricingSave(true)}
                        />
                      </td>
                      <td className={pricingTableBodyCellClass}>
                        <Input
                          type="number"
                          step="any"
                          className="h-9 px-3"
                          value={entry.inputPer1m}
                          onChange={(event) => handlePricingFieldChange(index, 'inputPer1m', event.target.value)}
                          onBlur={() => triggerPricingSave(true)}
                        />
                      </td>
                      <td className={pricingTableBodyCellClass}>
                        <Input
                          type="number"
                          step="any"
                          className="h-9 px-3"
                          value={entry.outputPer1m}
                          onChange={(event) => handlePricingFieldChange(index, 'outputPer1m', event.target.value)}
                          onBlur={() => triggerPricingSave(true)}
                        />
                      </td>
                      <td className={pricingTableBodyCellClass}>
                        <Input
                          type="number"
                          step="any"
                          className="h-9 px-3"
                          value={entry.cacheInputPer1m}
                          onChange={(event) => handlePricingFieldChange(index, 'cacheInputPer1m', event.target.value)}
                          onBlur={() => triggerPricingSave(true)}
                        />
                      </td>
                      <td className={pricingTableBodyCellClass}>
                        <Input
                          type="number"
                          step="any"
                          className="h-9 px-3"
                          value={entry.reasoningPer1m}
                          onChange={(event) => handlePricingFieldChange(index, 'reasoningPer1m', event.target.value)}
                          onBlur={() => triggerPricingSave(true)}
                        />
                      </td>
                      <td className={cn(pricingTableBodyCellClass, 'whitespace-nowrap')}>
                        <Badge variant={sourceBadgeVariant(entry.source)} className="inline-flex min-w-[5rem] justify-center">
                          {entry.source}
                        </Badge>
                      </td>
                      <td className={cn(pricingTableBodyCellClass, 'text-right whitespace-nowrap')}>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-8 px-2.5 text-error hover:bg-error/10"
                          onClick={() => handleRemovePricingEntry(index)}
                        >
                          {t('settings.pricing.remove')}
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="text-base-content/70">
                {isPricingSaving ? t('settings.saving') : t('settings.autoSaved')}
              </span>
              {pricingErrorKey && <span className="text-error">{t(pricingErrorKey)}</span>}
            </div>
          </CardContent>
        </Card>
      </div>

      <Card className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
        <CardHeader className="gap-2 border-b border-base-300/70 pb-4">
          <CardTitle>{t('settings.forwardProxy.title')}</CardTitle>
          <CardDescription>{t('settings.forwardProxy.description')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-5 pt-4">
          <label className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4 rounded-xl border border-base-300/75 bg-base-100/72 px-4 py-4">
            <div className="space-y-1">
              <div className="font-medium leading-snug">{t('settings.forwardProxy.insertDirectLabel')}</div>
              <div className="text-sm leading-snug text-base-content/70">{t('settings.forwardProxy.insertDirectHint')}</div>
            </div>
            <div className="pt-0.5">
              <Switch
                checked={forwardProxyInsertDirect}
                disabled={isForwardProxySaving}
                onCheckedChange={(checked) => {
                  setForwardProxyInsertDirect(checked)
                  setForwardProxyDirty(true)
                }}
              />
            </div>
          </label>

          <div className="flex flex-wrap items-center gap-3 rounded-xl border border-base-300/80 bg-base-100/72 px-3.5 py-3">
            <Button
              type="button"
              size="sm"
              variant="secondary"
              disabled={isForwardProxySaving}
              onClick={() => openForwardProxyAddModal('proxyBatch')}
            >
              <Icon icon="mdi:plus" className="mr-1 h-4 w-4" aria-hidden />
              {t('settings.forwardProxy.addProxyBatch')}
            </Button>
            <span className="text-xs text-base-content/70">{t('settings.forwardProxy.proxyCount', { count: forwardProxyUrls.length })}</span>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              disabled={isForwardProxySaving}
              onClick={() => openForwardProxyAddModal('subscriptionUrl')}
            >
              <Icon icon="mdi:plus" className="mr-1 h-4 w-4" aria-hidden />
              {t('settings.forwardProxy.addSubscription')}
            </Button>
            <span className="text-xs text-base-content/70">
              {t('settings.forwardProxy.subscriptionCount', { count: forwardProxySubscriptionUrls.length })}
            </span>
          </div>

          <div className="grid gap-3 lg:grid-cols-2">
            <section className="rounded-xl border border-base-300/80 bg-base-100/72 p-3">
              <header className="mb-2 flex items-center justify-between gap-2">
                <h4 className="text-sm font-medium text-base-content/85">{t('settings.forwardProxy.proxyUrls')}</h4>
                <span className="text-xs text-base-content/65">{forwardProxyUrls.length}</span>
              </header>
              {forwardProxyUrls.length === 0 ? (
                <p className="text-xs text-base-content/60">{t('settings.forwardProxy.listEmpty')}</p>
              ) : (
                <ul className="space-y-1.5">
                  {forwardProxyUrls.map((proxyUrl) => (
                    <li
                      key={`proxy-url-${proxyUrl}`}
                      className="flex items-center gap-2 rounded-lg border border-base-300/70 bg-base-100/75 px-2.5 py-1.5"
                    >
                      <code className="min-w-0 flex-1 truncate whitespace-nowrap text-[11px] tabular-nums" title={proxyUrl}>
                        {proxyUrl}
                      </code>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        className="h-7 px-2 text-xs text-error hover:bg-error/10 hover:text-error"
                        disabled={isForwardProxySaving}
                        onClick={() => handleRemoveForwardProxyUrl(proxyUrl)}
                      >
                        {t('settings.forwardProxy.remove')}
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="rounded-xl border border-base-300/80 bg-base-100/72 p-3">
              <header className="mb-2 flex items-center justify-between gap-2">
                <h4 className="text-sm font-medium text-base-content/85">{t('settings.forwardProxy.subscriptionUrls')}</h4>
                <span className="text-xs text-base-content/65">{forwardProxySubscriptionUrls.length}</span>
              </header>
              {forwardProxySubscriptionUrls.length === 0 ? (
                <p className="text-xs text-base-content/60">{t('settings.forwardProxy.subscriptionListEmpty')}</p>
              ) : (
                <ul className="space-y-1.5">
                  {forwardProxySubscriptionUrls.map((subscriptionUrl) => (
                    <li
                      key={`subscription-url-${subscriptionUrl}`}
                      className="flex items-center gap-2 rounded-lg border border-base-300/70 bg-base-100/75 px-2.5 py-1.5"
                    >
                      <code
                        className="min-w-0 flex-1 truncate whitespace-nowrap text-[11px] tabular-nums"
                        title={subscriptionUrl}
                      >
                        {subscriptionUrl}
                      </code>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        className="h-7 px-2 text-xs text-error hover:bg-error/10 hover:text-error"
                        disabled={isForwardProxySaving}
                        onClick={() => handleRemoveForwardProxySubscriptionUrl(subscriptionUrl)}
                      >
                        {t('settings.forwardProxy.remove')}
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </div>

          <div className="flex flex-wrap items-end gap-3">
            <div className="space-y-2">
              <label htmlFor="forward-proxy-interval" className="block text-sm font-medium text-base-content/75">
                {t('settings.forwardProxy.subscriptionInterval')}
              </label>
              <select
                id="forward-proxy-interval"
                className="h-10 min-w-48 rounded-xl border border-base-300/80 bg-base-100/70 px-3 text-sm outline-none transition focus:border-primary/50"
                value={forwardProxyIntervalSecs}
                onChange={(event) => {
                  setForwardProxyIntervalSecs(event.target.value)
                  setForwardProxyDirty(true)
                }}
              >
                {forwardProxyIntervalOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
            <Button
              type="button"
              className="h-10"
              disabled={isForwardProxySaving || !forwardProxyDirty}
              onClick={handleForwardProxySave}
            >
              {isForwardProxySaving ? t('settings.saving') : t('settings.forwardProxy.save')}
            </Button>
            <span className="text-xs text-base-content/70">{t('settings.forwardProxy.supportHint')}</span>
          </div>

          <div className="space-y-3 md:hidden">
            {forwardProxyTableNodes.map((node) => {
              const windows = [
                { label: t('settings.forwardProxy.table.oneMinute'), stats: node.stats.oneMinute },
                { label: t('settings.forwardProxy.table.fifteenMinutes'), stats: node.stats.fifteenMinutes },
                { label: t('settings.forwardProxy.table.oneHour'), stats: node.stats.oneHour },
                { label: t('settings.forwardProxy.table.oneDay'), stats: node.stats.oneDay },
                { label: t('settings.forwardProxy.table.sevenDays'), stats: node.stats.sevenDays },
              ]
              return (
                <article
                  key={`mobile-${node.key}`}
                  className={cn(
                    'rounded-xl border border-base-300/80 bg-base-100/72 p-3',
                    node.penalized ? 'bg-warning/8' : '',
                  )}
                >
                  <div className="flex items-center gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="truncate whitespace-nowrap text-sm font-medium" title={node.displayName}>
                        {node.displayName}
                      </div>
                    </div>
                    <div className="text-right">
                      <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {t('settings.forwardProxy.table.weight')}
                      </div>
                      <div className="font-mono text-sm tabular-nums">
                        {node.weight == null || Number.isNaN(node.weight) ? '—' : node.weight.toFixed(2)}
                      </div>
                    </div>
                  </div>
                  <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
                    {windows.map((window) => (
                      <div key={`${node.key}-${window.label}`} className="rounded-lg border border-base-300/70 bg-base-100/75 p-2">
                        <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">{window.label}</div>
                        <div className="mt-1 text-[11px] leading-tight">
                          <div>{formatSuccessRate(window.stats.successRate)}</div>
                          <div className="text-base-content/65">{formatLatency(window.stats.avgLatencyMs)}</div>
                        </div>
                      </div>
                    ))}
                  </div>
                </article>
              )
            })}
            {forwardProxyTableNodes.length === 0 && (
              <div className="rounded-xl border border-base-300/80 bg-base-100/72 px-4 py-6 text-center text-xs text-base-content/60">
                {t('settings.forwardProxy.table.empty')}
              </div>
            )}
          </div>

          <div className="hidden overflow-hidden rounded-xl border border-base-300/80 bg-base-100/72 md:block">
            <table className="w-full table-fixed text-xs">
              <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
                <tr>
                  <th className={cn('box-border w-[29%]', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.proxy')}</th>
                  <th className={cn('box-border w-[12%] text-center', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.oneMinute')}</th>
                  <th className={cn('box-border w-[12%] text-center', pricingTableHeaderCellClass)}>
                    {t('settings.forwardProxy.table.fifteenMinutes')}
                  </th>
                  <th className={cn('box-border w-[12%] text-center', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.oneHour')}</th>
                  <th className={cn('box-border w-[12%] text-center', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.oneDay')}</th>
                  <th className={cn('box-border w-[12%] text-center', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.sevenDays')}</th>
                  <th className={cn('box-border w-[11%] text-right', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.weight')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-base-300/65">
                {forwardProxyTableNodes.map((node) => {
                  const windows = [
                    node.stats.oneMinute,
                    node.stats.fifteenMinutes,
                    node.stats.oneHour,
                    node.stats.oneDay,
                    node.stats.sevenDays,
                  ]
                  return (
                    <tr key={node.key} className={cn('transition-colors hover:bg-primary/6', node.penalized ? 'bg-warning/8' : '')}>
                      <td className={cn(pricingTableBodyCellClass, 'box-border max-w-0 overflow-hidden px-3')}>
                        <div className="block w-full truncate whitespace-nowrap text-sm font-medium" title={node.displayName}>
                          {node.displayName}
                        </div>
                      </td>
                      {windows.map((window, index) => (
                        <td key={`${node.key}-${index}`} className={cn(pricingTableBodyCellClass, 'box-border px-2 text-center')}>
                          <div className="space-y-0.5 text-[11px] leading-tight">
                            <div>{formatSuccessRate(window.successRate)}</div>
                            <div className="text-base-content/65">{formatLatency(window.avgLatencyMs)}</div>
                          </div>
                        </td>
                      ))}
                      <td className={cn(pricingTableBodyCellClass, 'box-border whitespace-nowrap px-3 text-right font-mono text-sm')}>
                        {node.weight == null || Number.isNaN(node.weight) ? '—' : node.weight.toFixed(2)}
                      </td>
                    </tr>
                  )
                })}
                {forwardProxyTableNodes.length === 0 && (
                  <tr>
                    <td colSpan={7} className={cn(pricingTableBodyCellClass, 'py-6 text-center text-base-content/60')}>
                      {t('settings.forwardProxy.table.empty')}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>

          {forwardProxyModalKind &&
            typeof document !== 'undefined' &&
            createPortal(
              <div className="fixed inset-0 z-[80] flex items-center justify-center bg-base-content/45 p-4">
              <div className="w-full max-w-2xl rounded-2xl border border-base-300/75 bg-base-100 shadow-xl">
                <div className="space-y-1 border-b border-base-300/70 px-5 py-4">
                  <div className="flex items-start gap-3">
                    <div className="min-w-0 flex-1 space-y-1">
                      <h3 className="text-lg font-semibold">{forwardProxyModalTitle}</h3>
                      <p className="text-sm text-base-content/65">{t('settings.forwardProxy.modal.description')}</p>
                    </div>
                    {forwardProxyModalIsBatch && (
                      <ol className="hidden shrink-0 items-center gap-1 rounded-lg border border-base-300/70 bg-base-200/35 px-2 py-1 sm:flex">
                        {[1, 2].map((step) => {
                          const isActive = forwardProxyModalStep === step
                          const isCompleted = step === 1 && forwardProxyModalStep === 2
                          const canJump = step === 1 || forwardProxyModalStep === 2
                          return (
                            <li key={step} className="flex items-center gap-1.5">
                              <button
                                type="button"
                                className={cn(
                                  'flex items-center gap-1.5 rounded-md px-1 py-0.5 transition',
                                  canJump ? 'hover:bg-base-300/45' : 'cursor-default',
                                )}
                                disabled={!canJump}
                                onClick={() => {
                                  if (!canJump) return
                                  setForwardProxyModalStep(step as 1 | 2)
                                }}
                              >
                                <span
                                  className={cn(
                                    'inline-flex h-5 w-5 items-center justify-center rounded-full text-[10px] font-semibold',
                                    isActive
                                      ? 'bg-primary text-primary-content'
                                      : isCompleted
                                        ? 'bg-success/85 text-success-content'
                                        : 'bg-base-300/85 text-base-content/70',
                                  )}
                                >
                                  {isCompleted ? '✓' : step}
                                </span>
                                <span className={cn('text-[11px] font-medium', isActive ? 'text-primary' : 'text-base-content/65')}>
                                  {t(
                                    step === 1
                                      ? 'settings.forwardProxy.modal.step1Compact'
                                      : 'settings.forwardProxy.modal.step2Compact',
                                  )}
                                </span>
                              </button>
                              {step === 1 && <span className="text-base-content/35">/</span>}
                            </li>
                          )
                        })}
                      </ol>
                    )}
                  </div>
                  {forwardProxyModalIsBatch && (
                    <ol className="mt-2 flex items-center gap-1 rounded-lg border border-base-300/70 bg-base-200/35 px-2 py-1 sm:hidden">
                      {[1, 2].map((step) => {
                        const isActive = forwardProxyModalStep === step
                        const isCompleted = step === 1 && forwardProxyModalStep === 2
                        const canJump = step === 1 || forwardProxyModalStep === 2
                        return (
                          <li key={`mobile-${step}`} className="flex items-center gap-1.5">
                            <button
                              type="button"
                              className={cn(
                                'flex items-center gap-1.5 rounded-md px-1 py-0.5 transition',
                                canJump ? 'hover:bg-base-300/45' : 'cursor-default',
                              )}
                              disabled={!canJump}
                              onClick={() => {
                                if (!canJump) return
                                setForwardProxyModalStep(step as 1 | 2)
                              }}
                            >
                              <span
                                className={cn(
                                  'inline-flex h-5 w-5 items-center justify-center rounded-full text-[10px] font-semibold',
                                  isActive
                                    ? 'bg-primary text-primary-content'
                                    : isCompleted
                                      ? 'bg-success/85 text-success-content'
                                      : 'bg-base-300/85 text-base-content/70',
                                )}
                              >
                                {isCompleted ? '✓' : step}
                              </span>
                              <span className={cn('text-[11px] font-medium', isActive ? 'text-primary' : 'text-base-content/65')}>
                                {t(
                                  step === 1
                                    ? 'settings.forwardProxy.modal.step1Compact'
                                    : 'settings.forwardProxy.modal.step2Compact',
                                )}
                              </span>
                            </button>
                            {step === 1 && <span className="text-base-content/35">/</span>}
                          </li>
                        )
                      })}
                    </ol>
                  )}
                </div>
                <div className="space-y-4 px-5 py-4">
                  {(!forwardProxyModalIsBatch || forwardProxyModalStep === 1) && (
                    <div className="space-y-2">
                      <label htmlFor="forward-proxy-modal-input" className="block text-sm font-medium text-base-content/75">
                        {forwardProxyModalInputLabel}
                      </label>
                      {forwardProxyModalIsBatch ? (
                        <textarea
                          id="forward-proxy-modal-input"
                          className="h-36 w-full rounded-xl border border-base-300/80 bg-base-100/70 px-3 py-2 text-sm font-mono outline-none ring-0 transition focus:border-primary/50"
                          value={forwardProxyModalInput}
                          placeholder={forwardProxyModalPlaceholder}
                          onChange={(event) => {
                            forwardProxyBatchValidationRunRef.current += 1
                            setForwardProxyModalInput(event.target.value)
                            setForwardProxyBatchResults([])
                            setForwardProxyBatchTooltipKey(null)
                            setForwardProxyValidation({ status: 'idle' })
                          }}
                        />
                      ) : (
                        <Input
                          id="forward-proxy-modal-input"
                          value={forwardProxyModalInput}
                          placeholder={forwardProxyModalPlaceholder}
                          onChange={(event) => {
                            setForwardProxyModalInput(event.target.value)
                            setForwardProxyValidation({ status: 'idle' })
                          }}
                        />
                      )}
                    </div>
                  )}

                  {forwardProxyModalIsBatch && forwardProxyModalStep === 2 && (
                    <div className="space-y-3">
                      <Alert variant="info">
                        {forwardProxyBatchValidatingCount > 0
                          ? t('settings.forwardProxy.modal.batchValidateProgress', {
                              available: forwardProxyBatchAvailableCount,
                              unavailable: forwardProxyBatchUnavailableCount,
                              validating: forwardProxyBatchValidatingCount,
                            })
                          : t('settings.forwardProxy.modal.batchValidateSummary', {
                              available: forwardProxyBatchAvailableCount,
                              unavailable: forwardProxyBatchUnavailableCount,
                            })}
                      </Alert>
                      <div className="max-h-72 overflow-y-auto rounded-xl border border-base-300/75">
                        <table className="w-full table-fixed text-xs">
                          <thead className="bg-base-200/70 text-base-content/65">
                            <tr>
                              <th className="w-14 px-3 py-2 text-left">{t('settings.forwardProxy.modal.resultIndex')}</th>
                              <th className="px-3 py-2 text-left">{t('settings.forwardProxy.modal.resultName')}</th>
                              <th className="w-24 px-3 py-2 text-right">{t('settings.forwardProxy.modal.resultProtocol')}</th>
                              <th className="w-24 px-3 py-2 text-left">{t('settings.forwardProxy.modal.resultStatus')}</th>
                              <th className="w-32 px-3 py-2 text-right">{t('settings.forwardProxy.modal.resultAction')}</th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-base-300/65">
                            {forwardProxyBatchResults.map((item, index) => (
                              <tr key={item.key} className={item.status === 'unavailable' ? 'bg-warning/10' : ''}>
                                <td className="px-3 py-2 text-base-content/60">{index + 1}</td>
                                <td className="px-3 py-2">
                                  <div
                                    className="truncate text-sm font-medium text-base-content/85"
                                    title={item.displayName}
                                  >
                                    {item.displayName}
                                  </div>
                                </td>
                                <td className="px-3 py-2 text-right font-mono text-base-content/70">
                                  <span className="inline-block w-full truncate whitespace-nowrap text-sm">{item.protocolName}</span>
                                </td>
                                <td className="px-3 py-2">
                                  {(() => {
                                    const latestRound = item.lastRound
                                    const latestRoundLabel =
                                      latestRound == null
                                        ? t('settings.forwardProxy.modal.statusValidating')
                                        : latestRound.ok
                                          ? formatLatency(latestRound.latencyMs)
                                          : latestRound.timedOut
                                            ? t('settings.forwardProxy.modal.statusTimeout')
                                            : t('settings.forwardProxy.modal.statusUnavailable')
                                    const latestRoundTone =
                                      latestRound == null
                                        ? 'text-info'
                                        : latestRound.ok
                                          ? 'text-success'
                                          : latestRound.timedOut
                                            ? 'text-warning'
                                            : 'text-error'
                                    return (
                                      <div className="group relative inline-flex max-w-full flex-col items-start">
                                        <button
                                          type="button"
                                          className="text-left"
                                          disabled={item.rounds.length === 0}
                                          onClick={() => {
                                            if (item.rounds.length === 0) return
                                            setForwardProxyBatchTooltipKey((current) => (current === item.key ? null : item.key))
                                          }}
                                        >
                                          <span className={cn('inline-block whitespace-nowrap text-sm font-medium', latestRoundTone)}>
                                            {latestRoundLabel}
                                          </span>
                                          <span className="block font-mono text-[10px] text-base-content/65">
                                            {t('settings.forwardProxy.modal.roundProgress', {
                                              current: item.completedRounds,
                                              total: item.totalRounds,
                                            })}
                                          </span>
                                        </button>
                                        {item.rounds.length > 0 && (
                                          <div
                                            className={cn(
                                              'pointer-events-none absolute left-0 top-full z-20 mt-1 hidden max-h-52 min-w-[16rem] max-w-[22rem] overflow-y-auto rounded-md border border-base-300/80 bg-base-100/95 p-2 text-left text-[11px] leading-snug text-base-content shadow-lg',
                                              forwardProxyBatchTooltipKey === item.key ? 'block' : 'group-hover:block',
                                            )}
                                          >
                                            <div className="space-y-1 font-mono">
                                              {item.rounds.map((round) => (
                                                <div key={`${item.key}-round-${round.round}`}>
                                                  {round.ok
                                                    ? t('settings.forwardProxy.modal.roundResultSuccess', {
                                                        round: round.round,
                                                        latency: formatLatency(round.latencyMs),
                                                      })
                                                    : round.timedOut
                                                      ? t('settings.forwardProxy.modal.roundResultTimeout', { round: round.round })
                                                      : t('settings.forwardProxy.modal.roundResultFailed', { round: round.round })}
                                                </div>
                                              ))}
                                            </div>
                                          </div>
                                        )}
                                      </div>
                                    )
                                  })()}
                                </td>
                                <td className="px-3 py-2 text-right">
                                  <div className="inline-flex items-center gap-1.5">
                                    <Button
                                      type="button"
                                      size="icon"
                                      variant="ghost"
                                      className="h-8 w-8"
                                      title={t('settings.forwardProxy.modal.retryNode')}
                                      disabled={item.status === 'validating' || forwardProxyBatchValidatingCount > 0 || isForwardProxySaving}
                                      onClick={() => void handleRetryBatchNode(item.key)}
                                    >
                                      <Icon icon="mdi:refresh" className="h-4 w-4" aria-hidden />
                                      <span className="sr-only">{t('settings.forwardProxy.modal.retryNode')}</span>
                                    </Button>
                                    <Button
                                      type="button"
                                      size="icon"
                                      className="h-8 w-8"
                                      title={t('settings.forwardProxy.modal.addNode')}
                                      disabled={item.status !== 'available' || isForwardProxySaving}
                                      onClick={() => handleAddValidatedBatchNode(item.key)}
                                    >
                                      <Icon icon="mdi:plus" className="h-4 w-4" aria-hidden />
                                      <span className="sr-only">{t('settings.forwardProxy.modal.addNode')}</span>
                                    </Button>
                                  </div>
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    </div>
                  )}

                  {!forwardProxyModalIsBatch && forwardProxyValidation.status === 'validating' && (
                    <Alert variant="info">{t('settings.forwardProxy.modal.validating')}</Alert>
                  )}
                  {forwardProxyValidation.status === 'failed' && (
                    <Alert variant="error">{forwardProxyValidation.message}</Alert>
                  )}
                  {!forwardProxyModalIsBatch && forwardProxyValidation.status === 'passed' && (
                    <Alert variant="success">
                      <div className="space-y-1">
                        <div>{forwardProxyValidation.message || t('settings.forwardProxy.modal.validateSuccess')}</div>
                        {forwardProxyValidation.normalizedValues.slice(0, 2).map((value) => (
                          <div key={value} className="font-mono text-xs opacity-80">
                            {t('settings.forwardProxy.modal.normalizedValue', { value })}
                          </div>
                        ))}
                        {(forwardProxyValidation.discoveredNodes != null || forwardProxyValidation.latencyMs != null) && (
                          <div className="text-xs opacity-80">
                            {t('settings.forwardProxy.modal.probeSummary', {
                              nodes: forwardProxyValidation.discoveredNodes ?? 1,
                              latency:
                                forwardProxyValidation.latencyMs == null
                                  ? '—'
                                  : `${forwardProxyValidation.latencyMs.toFixed(0)} ms`,
                            })}
                          </div>
                        )}
                      </div>
                    </Alert>
                  )}
                </div>
                <div className="flex items-center justify-end gap-2 border-t border-base-300/70 px-5 py-3">
                  <Button type="button" variant="ghost" onClick={closeForwardProxyAddModal}>
                    {t('settings.forwardProxy.modal.cancel')}
                  </Button>
                  {forwardProxyModalIsBatch && forwardProxyModalStep === 2 ? (
                    <Button type="button" disabled={!forwardProxyCanSubmitBatch} onClick={handleSubmitValidatedBatchNodes}>
                      {t('settings.forwardProxy.modal.submitWithCount', { count: forwardProxyBatchAvailableCount })}
                    </Button>
                  ) : (
                    <Button
                      type="button"
                      variant="secondary"
                      disabled={forwardProxyValidation.status === 'validating'}
                      onClick={() => void handleValidateForwardProxyCandidate()}
                    >
                      {t('settings.forwardProxy.modal.validate')}
                    </Button>
                  )}
                  {!forwardProxyModalIsBatch && (
                    <Button type="button" disabled={!forwardProxyCanConfirmAdd} onClick={handleConfirmAddForwardProxyCandidate}>
                      {t('settings.forwardProxy.modal.add')}
                    </Button>
                  )}
                </div>
              </div>
            </div>,
              document.body,
            )}
        </CardContent>
      </Card>

      {error && <Alert variant="error" className="text-sm">{t('settings.loadError', { error })}</Alert>}
    </section>
  )
}
