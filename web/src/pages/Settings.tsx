import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { Badge } from '../components/ui/badge'
import { Alert } from '../components/ui/alert'
import { Button } from '../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card'
import { Input } from '../components/ui/input'
import { Switch } from '../components/ui/switch'
import { useSettings } from '../hooks/useSettings'
import type { PricingEntry, PricingSettings } from '../lib/api'
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

export default function SettingsPage() {
  const { t } = useTranslation()
  const {
    settings,
    isLoading,
    isProxySaving,
    isPricingSaving,
    pricingRollbackVersion,
    error,
    saveProxy,
    savePricing,
  } = useSettings()

  const [pricingDraft, setPricingDraft] = useState<PricingDraft | null>(null)
  const [pricingErrorKey, setPricingErrorKey] = useState<string | null>(null)
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
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

  const currentProxy = settings?.proxy ?? null
  const enabledPresetModelSet = useMemo(
    () => new Set(currentProxy?.enabledModels ?? []),
    [currentProxy?.enabledModels],
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

  if (isLoading) {
    return (
      <section className="mx-auto max-w-6xl space-y-4">
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="text-sm text-base-content/70">{t('settings.loading')}</p>
      </section>
    )
  }

  if (!settings || !currentProxy) {
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

      {error && <Alert variant="error" className="text-sm">{t('settings.loadError', { error })}</Alert>}
    </section>
  )
}
