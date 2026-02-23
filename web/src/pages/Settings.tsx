import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { useSettings } from '../hooks/useSettings'
import type { PricingEntry, PricingSettings } from '../lib/api'
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

function sourceBadgeClass(source: string): string {
  if (source === 'official') return 'badge-success'
  if (source === 'temporary') return 'badge-warning'
  return 'badge-ghost'
}

export default function SettingsPage() {
  const { t } = useTranslation()
  const {
    settings,
    isLoading,
    isProxySaving,
    isPricingSaving,
    error,
    saveProxy,
    savePricing,
  } = useSettings()

  const [pricingDraft, setPricingDraft] = useState<PricingDraft | null>(null)
  const [pricingErrorKey, setPricingErrorKey] = useState<string | null>(null)
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    if (!settings?.pricing) return
    setPricingDraft(toPricingDraft(settings.pricing))
  }, [settings?.pricing])

  const currentProxy = settings?.proxy ?? null
  const enabledPresetModelSet = useMemo(
    () => new Set(currentProxy?.enabledModels ?? []),
    [currentProxy?.enabledModels],
  )

  const remotePricingKey = useMemo(() => stablePricingKey(settings?.pricing ?? null), [settings?.pricing])

  const triggerPricingSave = useCallback(
    (forceImmediate: boolean) => {
      if (!settings || !pricingDraft) return
      const parsed = parsePricingDraft(pricingDraft)
      if (!parsed.value) {
        setPricingErrorKey(parsed.error ?? 'settings.pricing.errors.numberInvalid')
        return
      }
      const draftKey = stablePricingKey(parsed.value)
      if (draftKey === remotePricingKey) {
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
    [pricingDraft, remotePricingKey, savePricing, settings],
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

  if (!settings || !currentProxy || !pricingDraft) {
    return (
      <section className="mx-auto max-w-6xl space-y-4">
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="text-sm text-error">{t('settings.loadError', { error: error ?? 'unknown' })}</p>
      </section>
    )
  }

  return (
    <section className="mx-auto max-w-6xl space-y-6">
      <div>
        <h1 className="text-2xl font-semibold">{t('settings.title')}</h1>
        <p className="mt-1 text-sm text-base-content/70">{t('settings.description')}</p>
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <article className="rounded-box border border-base-300 bg-base-100 p-4 shadow-sm">
          <h2 className="text-lg font-medium">{t('settings.proxy.title')}</h2>
          <p className="mt-1 text-sm text-base-content/70">{t('settings.proxy.description')}</p>

          <div className="mt-4 space-y-3">
            <label className="flex items-start justify-between gap-3 rounded-lg border border-base-300 p-3">
              <div>
                <div className="font-medium">{t('settings.proxy.hijackLabel')}</div>
                <div className="text-sm text-base-content/70">{t('settings.proxy.hijackHint')}</div>
              </div>
              <input
                type="checkbox"
                className="toggle toggle-primary"
                checked={currentProxy.hijackEnabled}
                disabled={isProxySaving}
                onChange={handleToggleHijack}
              />
            </label>

            <label className="flex items-start justify-between gap-3 rounded-lg border border-base-300 p-3">
              <div>
                <div className="font-medium">{t('settings.proxy.mergeLabel')}</div>
                <div className="text-sm text-base-content/70">{t('settings.proxy.mergeHint')}</div>
                {!currentProxy.hijackEnabled && (
                  <div className="mt-1 text-xs text-warning">{t('settings.proxy.mergeDisabledHint')}</div>
                )}
              </div>
              <input
                type="checkbox"
                className="toggle toggle-primary"
                checked={currentProxy.mergeUpstreamEnabled}
                disabled={isProxySaving || !currentProxy.hijackEnabled}
                onChange={handleToggleMergeUpstream}
              />
            </label>

            <div className="rounded-lg border border-base-300 p-3">
              <div className="mb-2 flex items-center justify-between gap-2">
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
                      className={`flex cursor-pointer items-center justify-between rounded-lg border px-3 py-2 ${
                        enabled ? 'border-primary/50 bg-primary/5' : 'border-base-300 bg-base-100'
                      } ${isProxySaving ? 'cursor-not-allowed opacity-70' : ''}`}
                    >
                      <span className="truncate font-mono text-sm">{modelId}</span>
                      <input
                        type="checkbox"
                        className="checkbox checkbox-sm checkbox-primary"
                        checked={enabled}
                        disabled={isProxySaving}
                        onChange={() => handleTogglePresetModel(modelId)}
                      />
                    </label>
                  )
                })}
              </div>
              {currentProxy.enabledModels.length === 0 && (
                <div className="mt-2 text-xs text-warning">{t('settings.proxy.noneEnabledHint')}</div>
              )}
            </div>

            <div className="text-xs text-base-content/70">{isProxySaving ? t('settings.saving') : t('settings.autoSaved')}</div>
          </div>
        </article>

        <article className="rounded-box border border-base-300 bg-base-100 p-4 shadow-sm">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 className="text-lg font-medium">{t('settings.pricing.title')}</h2>
              <p className="mt-1 text-sm text-base-content/70">{t('settings.pricing.description')}</p>
            </div>
            <button type="button" className="btn btn-sm btn-primary" onClick={handleAddPricingEntry}>
              <Icon icon="mdi:plus" className="h-4 w-4" aria-hidden />
              {t('settings.pricing.add')}
            </button>
          </div>

          <label className="form-control mt-4">
            <div className="label py-1">
              <span className="label-text">{t('settings.pricing.catalogVersion')}</span>
            </div>
            <input
              type="text"
              className="input input-bordered input-sm"
              value={pricingDraft.catalogVersion}
              onChange={(event) => handleCatalogVersionChange(event.target.value)}
              onBlur={() => triggerPricingSave(true)}
            />
          </label>

          <div className="mt-4 overflow-x-auto">
            <table className="table table-zebra table-sm">
              <thead>
                <tr>
                  <th>{t('settings.pricing.columns.model')}</th>
                  <th>{t('settings.pricing.columns.input')}</th>
                  <th>{t('settings.pricing.columns.output')}</th>
                  <th>{t('settings.pricing.columns.cacheInput')}</th>
                  <th>{t('settings.pricing.columns.reasoning')}</th>
                  <th>{t('settings.pricing.columns.source')}</th>
                  <th>{t('settings.pricing.columns.actions')}</th>
                </tr>
              </thead>
              <tbody>
                {pricingDraft.entries.map((entry, index) => (
                  <tr key={`${entry.model}-${index}`}>
                    <td>
                      <input
                        type="text"
                        className="input input-bordered input-xs w-44"
                        value={entry.model}
                        onChange={(event) => handlePricingFieldChange(index, 'model', event.target.value)}
                        onBlur={() => triggerPricingSave(true)}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        step="any"
                        className="input input-bordered input-xs w-28"
                        value={entry.inputPer1m}
                        onChange={(event) => handlePricingFieldChange(index, 'inputPer1m', event.target.value)}
                        onBlur={() => triggerPricingSave(true)}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        step="any"
                        className="input input-bordered input-xs w-28"
                        value={entry.outputPer1m}
                        onChange={(event) => handlePricingFieldChange(index, 'outputPer1m', event.target.value)}
                        onBlur={() => triggerPricingSave(true)}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        step="any"
                        className="input input-bordered input-xs w-28"
                        value={entry.cacheInputPer1m}
                        onChange={(event) => handlePricingFieldChange(index, 'cacheInputPer1m', event.target.value)}
                        onBlur={() => triggerPricingSave(true)}
                      />
                    </td>
                    <td>
                      <input
                        type="number"
                        step="any"
                        className="input input-bordered input-xs w-28"
                        value={entry.reasoningPer1m}
                        onChange={(event) => handlePricingFieldChange(index, 'reasoningPer1m', event.target.value)}
                        onBlur={() => triggerPricingSave(true)}
                      />
                    </td>
                    <td>
                      <span className={`badge badge-sm ${sourceBadgeClass(entry.source)}`}>{entry.source}</span>
                    </td>
                    <td>
                      <button
                        type="button"
                        className="btn btn-ghost btn-xs text-error"
                        onClick={() => handleRemovePricingEntry(index)}
                      >
                        {t('settings.pricing.remove')}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
            <span className="text-base-content/70">
              {isPricingSaving ? t('settings.saving') : t('settings.autoSaved')}
            </span>
            {pricingErrorKey && <span className="text-error">{t(pricingErrorKey)}</span>}
          </div>
        </article>
      </div>

      {error && <div className="alert alert-error text-sm">{t('settings.loadError', { error })}</div>}
    </section>
  )
}
