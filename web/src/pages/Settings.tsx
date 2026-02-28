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
}

const AUTO_SAVE_DEBOUNCE_MS = 600
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

function batchStatusRank(status: ForwardProxyBatchValidationStatus): number {
  if (status === 'available') return 3
  if (status === 'validating') return 2
  return 1
}

function pickPreferredBatchItem(
  existing: ForwardProxyBatchValidationItem,
  candidate: ForwardProxyBatchValidationItem,
): ForwardProxyBatchValidationItem {
  const existingRank = batchStatusRank(existing.status)
  const candidateRank = batchStatusRank(candidate.status)
  if (candidateRank > existingRank) return { ...candidate, order: Math.min(existing.order, candidate.order) }
  if (candidateRank < existingRank) return existing

  if (
    existing.status === 'available' &&
    candidate.status === 'available' &&
    candidate.latencyMs != null &&
    (existing.latencyMs == null || candidate.latencyMs < existing.latencyMs)
  ) {
    return { ...candidate, order: Math.min(existing.order, candidate.order) }
  }
  return existing
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

  const openForwardProxyAddModal = useCallback((kind: ForwardProxyModalKind) => {
    forwardProxyBatchValidationRunRef.current += 1
    setForwardProxyModalKind(kind)
    setForwardProxyModalStep(1)
    setForwardProxyModalInput('')
    setForwardProxyBatchResults([])
    setForwardProxyValidation({ status: 'idle' })
  }, [])

  const closeForwardProxyAddModal = useCallback(() => {
    forwardProxyBatchValidationRunRef.current += 1
    setForwardProxyModalKind(null)
    setForwardProxyModalStep(1)
    setForwardProxyModalInput('')
    setForwardProxyBatchResults([])
    setForwardProxyValidation({ status: 'idle' })
  }, [])

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
          message: err instanceof Error ? err.message : String(err),
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

    const resultMap = new Map<string, ForwardProxyBatchValidationItem>()
    const pendingKeyByRaw = new Map<string, string>()
    for (const [index, rawLine] of lines.entries()) {
      const pendingKey = `__pending__${index}`
      pendingKeyByRaw.set(rawLine, pendingKey)
      resultMap.set(pendingKey, {
        key: pendingKey,
        order: index,
        rawValue: rawLine,
        normalizedValue: rawLine,
        displayName: extractProxyDisplayName(rawLine) || unknownNodeName,
        protocolName: extractProxyProtocolName(rawLine) || unknownProtocolName,
        status: 'validating',
        message: t('settings.forwardProxy.modal.rowValidating'),
      })
    }

    setForwardProxyBatchResults(sortBatchResults(resultMap.values()))
    setForwardProxyValidation({ status: 'validating' })

    for (const [lineIndex, rawLine] of lines.entries()) {
      if (forwardProxyBatchValidationRunRef.current !== validationRunId) return

      let item: ForwardProxyBatchValidationItem
      try {
        const result = await validateForwardProxyCandidate({
          kind: 'proxyUrl',
          value: rawLine,
        })
        if (result.ok) {
          const normalizedValue = result.normalizedValue?.trim() || rawLine
          item = {
            key: normalizedValue,
            order: lineIndex,
            rawValue: rawLine,
            normalizedValue,
            displayName: extractProxyDisplayName(normalizedValue) || extractProxyDisplayName(rawLine) || unknownNodeName,
            protocolName: extractProxyProtocolName(normalizedValue) || extractProxyProtocolName(rawLine) || unknownProtocolName,
            status: 'available',
            latencyMs: result.latencyMs,
            message: result.message || t('settings.forwardProxy.modal.validateSuccess'),
          }
        } else {
          item = {
            key: rawLine,
            order: lineIndex,
            rawValue: rawLine,
            normalizedValue: rawLine,
            displayName: extractProxyDisplayName(rawLine) || unknownNodeName,
            protocolName: extractProxyProtocolName(rawLine) || unknownProtocolName,
            status: 'unavailable',
            latencyMs: result.latencyMs,
            message: t('settings.forwardProxy.modal.validateFailed'),
          }
        }
      } catch {
        item = {
          key: rawLine,
          order: lineIndex,
          rawValue: rawLine,
          normalizedValue: rawLine,
          displayName: extractProxyDisplayName(rawLine) || unknownNodeName,
          protocolName: extractProxyProtocolName(rawLine) || unknownProtocolName,
          status: 'unavailable',
          message: t('settings.forwardProxy.modal.validateFailed'),
        }
      }

      resultMap.delete(pendingKeyByRaw.get(rawLine) ?? rawLine)
      const existing = resultMap.get(item.key)
      if (!existing) {
        resultMap.set(item.key, item)
      } else {
        resultMap.set(item.key, pickPreferredBatchItem(existing, item))
      }

      if (forwardProxyBatchValidationRunRef.current !== validationRunId) return

      const progressResults = sortBatchResults(resultMap.values())
      const availableCount = progressResults.filter((entry) => entry.status === 'available').length
      const unavailableCount = progressResults.filter((entry) => entry.status === 'unavailable').length
      const validatingCount = progressResults.length - availableCount - unavailableCount

      setForwardProxyBatchResults(progressResults)
      setForwardProxyValidation({
        status: 'validating',
        message: t('settings.forwardProxy.modal.batchValidateProgress', {
          available: availableCount,
          unavailable: unavailableCount,
          validating: validatingCount,
        }),
      })
    }

    if (forwardProxyBatchValidationRunRef.current !== validationRunId) return

    const results = sortBatchResults(resultMap.values())
    const availableCount = results.filter((item) => item.status === 'available').length
    const unavailableCount = results.filter((item) => item.status === 'unavailable').length

    setForwardProxyBatchResults(results)
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
  }, [forwardProxyModalInput, forwardProxyModalKind, t])

  const handleConfirmAddForwardProxyCandidate = useCallback(() => {
    if (forwardProxyValidation.status !== 'passed' || !forwardProxyModalKind) return
    if (forwardProxyModalKind !== 'subscriptionUrl') return
    if (forwardProxyValidation.normalizedValues.length === 0) return
    setForwardProxySubscriptionUrls((current) => appendUniqueItem(current, forwardProxyValidation.normalizedValues[0]))
    setForwardProxyDirty(true)
    closeForwardProxyAddModal()
  }, [closeForwardProxyAddModal, forwardProxyModalKind, forwardProxyValidation])

  const handleRetryBatchNode = useCallback(
    async (nodeKey: string) => {
      if (forwardProxyModalKind !== 'proxyBatch') return
      const target = forwardProxyBatchResults.find((item) => item.key === nodeKey)
      if (!target) return
      const unknownNodeName = t('settings.forwardProxy.modal.unknownNode')
      const unknownProtocolName = t('settings.forwardProxy.modal.unknownProtocol')

      setForwardProxyBatchResults((current) =>
        current.map((item) =>
          item.key === nodeKey
            ? {
                ...item,
                status: 'validating',
                latencyMs: undefined,
                message: t('settings.forwardProxy.modal.rowValidating'),
              }
            : item,
        ),
      )

      try {
        const result = await validateForwardProxyCandidate({
          kind: 'proxyUrl',
          value: target.rawValue,
        })

        setForwardProxyBatchResults((current) =>
          current.map((item) => {
            if (item.key !== nodeKey) return item
            if (!result.ok) {
              return {
                ...item,
                status: 'unavailable',
                latencyMs: result.latencyMs,
                message: t('settings.forwardProxy.modal.validateFailed'),
              }
            }

            const normalizedValue = result.normalizedValue?.trim() || target.rawValue
            return {
              ...item,
              rawValue: target.rawValue,
              normalizedValue,
              displayName:
                extractProxyDisplayName(normalizedValue) ||
                extractProxyDisplayName(target.rawValue) ||
                item.displayName ||
                unknownNodeName,
              protocolName:
                extractProxyProtocolName(normalizedValue) ||
                extractProxyProtocolName(target.rawValue) ||
                item.protocolName ||
                unknownProtocolName,
              status: 'available',
              latencyMs: result.latencyMs,
              message: result.message || t('settings.forwardProxy.modal.validateSuccess'),
            }
          }),
        )
      } catch {
        setForwardProxyBatchResults((current) =>
          current.map((item) =>
            item.key === nodeKey
              ? {
                  ...item,
                  status: 'unavailable',
                  message: t('settings.forwardProxy.modal.validateFailed'),
                }
              : item,
          ),
        )
      }
    },
    [forwardProxyBatchResults, forwardProxyModalKind, t],
  )

  const handleAddValidatedBatchNode = useCallback(
    (nodeKey: string) => {
      if (forwardProxyModalKind !== 'proxyBatch') return
      setForwardProxyBatchResults((current) => {
        const target = current.find((item) => item.key === nodeKey)
        if (!target || target.status !== 'available') return current
        setForwardProxyUrls((list) => appendUniqueItem(list, target.normalizedValue))
        setForwardProxyDirty(true)
        const next = current.filter((item) => item.key !== nodeKey)
        if (next.length === 0) {
          queueMicrotask(() => closeForwardProxyAddModal())
        }
        return next
      })
    },
    [closeForwardProxyAddModal, forwardProxyModalKind],
  )

  const handleSubmitValidatedBatchNodes = useCallback(() => {
    if (forwardProxyModalKind !== 'proxyBatch') return
    setForwardProxyBatchResults((current) => {
      const availableItems = current.filter((item) => item.status === 'available')
      if (availableItems.length === 0) return current
      setForwardProxyUrls((list) => appendUniqueItems(list, availableItems.map((item) => item.normalizedValue)))
      setForwardProxyDirty(true)
      const next = current.filter((item) => item.status !== 'available')
      if (next.length === 0) {
        queueMicrotask(() => closeForwardProxyAddModal())
      }
      return next
    })
  }, [closeForwardProxyAddModal, forwardProxyModalKind])

  const removeForwardProxyUrl = useCallback((target: string) => {
    setForwardProxyUrls((current) => current.filter((item) => item !== target))
    setForwardProxyDirty(true)
  }, [])

  const removeForwardProxySubscriptionUrl = useCallback((target: string) => {
    setForwardProxySubscriptionUrls((current) => current.filter((item) => item !== target))
    setForwardProxyDirty(true)
  }, [])

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
  const forwardProxyCanConfirmAdd =
    !forwardProxyModalIsBatch &&
    forwardProxyValidation.status === 'passed' &&
    forwardProxyValidation.normalizedValues.length > 0 &&
    !isForwardProxySaving
  const forwardProxyCanSubmitBatch =
    forwardProxyModalIsBatch &&
    forwardProxyModalStep === 2 &&
    forwardProxyBatchAvailableCount > 0 &&
    forwardProxyBatchValidatingCount === 0 &&
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

          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-xl border border-base-300/80 bg-base-100/72 p-3.5">
              <div className="mb-3 flex items-center justify-between gap-2">
                <div className="text-sm font-medium text-base-content/75">{t('settings.forwardProxy.proxyUrls')}</div>
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
              </div>
              <div className="max-h-44 space-y-2 overflow-y-auto pr-1">
                {forwardProxyUrls.map((proxyUrl) => (
                  <div key={proxyUrl} className="flex items-start gap-2 rounded-lg border border-base-300/80 bg-base-100/70 px-2.5 py-2">
                    <div className="min-w-0 flex-1 font-mono text-xs text-base-content/85">{proxyUrl}</div>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2 text-error hover:bg-error/10"
                      disabled={isForwardProxySaving}
                      onClick={() => removeForwardProxyUrl(proxyUrl)}
                    >
                      {t('settings.forwardProxy.remove')}
                    </Button>
                  </div>
                ))}
                {forwardProxyUrls.length === 0 && (
                  <div className="rounded-lg border border-dashed border-base-300/80 px-3 py-4 text-xs text-base-content/60">
                    {t('settings.forwardProxy.listEmpty')}
                  </div>
                )}
              </div>
            </div>

            <div className="rounded-xl border border-base-300/80 bg-base-100/72 p-3.5">
              <div className="mb-3 flex items-center justify-between gap-2">
                <div className="text-sm font-medium text-base-content/75">{t('settings.forwardProxy.subscriptionUrls')}</div>
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
              </div>
              <div className="max-h-44 space-y-2 overflow-y-auto pr-1">
                {forwardProxySubscriptionUrls.map((subscriptionUrl) => (
                  <div
                    key={subscriptionUrl}
                    className="flex items-start gap-2 rounded-lg border border-base-300/80 bg-base-100/70 px-2.5 py-2"
                  >
                    <div className="min-w-0 flex-1 break-all text-xs text-base-content/85">{subscriptionUrl}</div>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="h-7 px-2 text-error hover:bg-error/10"
                      disabled={isForwardProxySaving}
                      onClick={() => removeForwardProxySubscriptionUrl(subscriptionUrl)}
                    >
                      {t('settings.forwardProxy.remove')}
                    </Button>
                  </div>
                ))}
                {forwardProxySubscriptionUrls.length === 0 && (
                  <div className="rounded-lg border border-dashed border-base-300/80 px-3 py-4 text-xs text-base-content/60">
                    {t('settings.forwardProxy.listEmpty')}
                  </div>
                )}
              </div>
            </div>
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

          <div className="overflow-x-auto rounded-xl border border-base-300/80 bg-base-100/72">
            <table className="w-full min-w-[72rem] table-fixed text-xs">
              <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
                <tr>
                  <th className={cn('w-56', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.proxy')}</th>
                  <th className={pricingTableHeaderCellClass}>{t('settings.forwardProxy.table.oneMinute')}</th>
                  <th className={pricingTableHeaderCellClass}>{t('settings.forwardProxy.table.fifteenMinutes')}</th>
                  <th className={pricingTableHeaderCellClass}>{t('settings.forwardProxy.table.oneHour')}</th>
                  <th className={pricingTableHeaderCellClass}>{t('settings.forwardProxy.table.oneDay')}</th>
                  <th className={pricingTableHeaderCellClass}>{t('settings.forwardProxy.table.sevenDays')}</th>
                  <th className={cn('w-28 text-right', pricingTableHeaderCellClass)}>{t('settings.forwardProxy.table.weight')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-base-300/65">
                {currentForwardProxy.nodes.map((node) => {
                  const windows = [
                    node.stats.oneMinute,
                    node.stats.fifteenMinutes,
                    node.stats.oneHour,
                    node.stats.oneDay,
                    node.stats.sevenDays,
                  ]
                  return (
                    <tr key={node.key} className={cn('transition-colors hover:bg-primary/6', node.penalized ? 'bg-warning/8' : '')}>
                      <td className={pricingTableBodyCellClass}>
                        <div className="space-y-0.5">
                          <div className="font-medium text-sm">{node.displayName}</div>
                          <div className="font-mono text-[11px] text-base-content/65">
                            {node.endpointUrl ?? t('settings.forwardProxy.directLabel')}
                          </div>
                        </div>
                      </td>
                      {windows.map((window, index) => (
                        <td key={`${node.key}-${index}`} className={pricingTableBodyCellClass}>
                          <div className="space-y-0.5 text-[11px] leading-tight">
                            <div>{t('settings.forwardProxy.table.successRate', { value: formatSuccessRate(window.successRate) })}</div>
                            <div className="text-base-content/65">
                              {t('settings.forwardProxy.table.avgLatency', { value: formatLatency(window.avgLatencyMs) })}
                            </div>
                          </div>
                        </td>
                      ))}
                      <td className={cn(pricingTableBodyCellClass, 'text-right font-mono text-sm')}>
                        {node.weight.toFixed(2)}
                      </td>
                    </tr>
                  )
                })}
                {currentForwardProxy.nodes.length === 0 && (
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
                                  <span
                                    className={cn(
                                      'inline-block whitespace-nowrap text-sm font-medium',
                                      item.status === 'available'
                                        ? 'text-success'
                                        : item.status === 'unavailable'
                                          ? 'text-warning'
                                          : 'text-info',
                                    )}
                                  >
                                    {item.status === 'available'
                                      ? formatLatency(item.latencyMs)
                                      : item.status === 'unavailable'
                                        ? t('settings.forwardProxy.modal.statusUnavailable')
                                        : t('settings.forwardProxy.modal.statusValidating')}
                                  </span>
                                </td>
                                <td className="px-3 py-2 text-right">
                                  <div className="inline-flex items-center gap-1.5">
                                    <Button
                                      type="button"
                                      size="icon"
                                      variant="ghost"
                                      className="h-8 w-8"
                                      title={t('settings.forwardProxy.modal.retryNode')}
                                      disabled={item.status === 'validating' || isForwardProxySaving}
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
