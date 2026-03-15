import { useMemo, useState } from 'react'
import { AppIcon } from '../../components/AppIcon'
import { Alert } from '../../components/ui/alert'
import { Badge } from '../../components/ui/badge'
import { Button } from '../../components/ui/button'
import { Input } from '../../components/ui/input'
import { Spinner } from '../../components/ui/spinner'
import { TagRuleDialog } from '../../components/TagRuleDialog'
import { usePoolTags } from '../../hooks/usePoolTags'
import type { CreateTagPayload, TagSummary, UpdateTagPayload } from '../../lib/api'
import { useTranslation } from '../../i18n'

type TernaryFilter = 'all' | 'true' | 'false'

function toBooleanQuery(value: TernaryFilter) {
  if (value === 'all') return undefined
  return value === 'true'
}

function RuleBadge({
  label,
  variant,
}: {
  label: string
  variant: 'default' | 'info' | 'accent'
}) {
  return (
    <Badge variant={variant} className="px-2.5 py-1 text-[11px] font-semibold">
      {label}
    </Badge>
  )
}

export default function TagsPage() {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const [hasAccountsFilter, setHasAccountsFilter] = useState<TernaryFilter>('all')
  const [guardEnabledFilter, setGuardEnabledFilter] = useState<TernaryFilter>('all')
  const [cutOutFilter, setCutOutFilter] = useState<TernaryFilter>('all')
  const [cutInFilter, setCutInFilter] = useState<TernaryFilter>('all')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogMode, setDialogMode] = useState<'create' | 'edit'>('create')
  const [activeTag, setActiveTag] = useState<TagSummary | null>(null)
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const query = useMemo(
    () => ({
      search,
      hasAccounts: toBooleanQuery(hasAccountsFilter),
      guardEnabled: toBooleanQuery(guardEnabledFilter),
      allowCutOut: cutOutFilter === 'all' ? undefined : cutOutFilter === 'true',
      allowCutIn: cutInFilter === 'all' ? undefined : cutInFilter === 'true',
    }),
    [cutInFilter, cutOutFilter, guardEnabledFilter, hasAccountsFilter, search],
  )

  const { items, writesEnabled, isLoading, error, updateQuery, createTag, updateTag, deleteTag } = usePoolTags(query)

  const openCreate = () => {
    setDialogMode('create')
    setActiveTag(null)
    setDialogError(null)
    setDialogOpen(true)
  }

  const openEdit = (tag: TagSummary) => {
    setDialogMode('edit')
    setActiveTag(tag)
    setDialogError(null)
    setDialogOpen(true)
  }

  const handleSubmit = async (payload: CreateTagPayload | UpdateTagPayload) => {
    setBusy(true)
    setDialogError(null)
    try {
      if (dialogMode === 'create') {
        await createTag(payload as CreateTagPayload)
      } else if (activeTag) {
        await updateTag(activeTag.id, payload)
      }
      setDialogOpen(false)
      setActiveTag(null)
    } catch (err) {
      setDialogError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="section-heading">
              <h2 className="section-title">{t('accountPool.tags.title')}</h2>
              <p className="section-description">{t('accountPool.tags.description')}</p>
            </div>
            <div className="flex flex-wrap gap-2">
              {writesEnabled ? (
                <Button type="button" onClick={openCreate}>
                  <AppIcon name="tag-plus-outline" className="mr-2 h-4 w-4" aria-hidden />
                  {t('accountPool.tags.actions.create')}
                </Button>
              ) : null}
            </div>
          </div>

          {error ? (
            <Alert variant="error">
              <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{error}</div>
            </Alert>
          ) : null}

          <div className="grid gap-3 lg:grid-cols-2 xl:grid-cols-[minmax(0,1.05fr)_repeat(4,minmax(12rem,1fr))]">
            <label className="field">
              <span className="field-label">{t('accountPool.tags.filters.search')}</span>
              <Input
                name="tagSearch"
                value={search}
                placeholder={t('accountPool.tags.filters.searchPlaceholder')}
                onChange={(event) => {
                  const value = event.target.value
                  setSearch(value)
                  updateQuery({ ...query, search: value })
                }}
              />
            </label>
            <label className="field">
              <span className="field-label">{t('accountPool.tags.filters.hasAccounts')}</span>
              <select
                name="tagHasAccountsFilter"
                className="field-select"
                value={hasAccountsFilter}
                onChange={(event) => {
                  const value = event.target.value as TernaryFilter
                  setHasAccountsFilter(value)
                  updateQuery({ ...query, hasAccounts: toBooleanQuery(value) })
                }}
              >
                <option value="all">{t('accountPool.tags.filters.option.all')}</option>
                <option value="true">{t('accountPool.tags.filters.option.linked')}</option>
                <option value="false">{t('accountPool.tags.filters.option.unlinked')}</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">{t('accountPool.tags.filters.guardEnabled')}</span>
              <select
                name="tagGuardFilter"
                className="field-select"
                value={guardEnabledFilter}
                onChange={(event) => {
                  const value = event.target.value as TernaryFilter
                  setGuardEnabledFilter(value)
                  updateQuery({ ...query, guardEnabled: toBooleanQuery(value) })
                }}
              >
                <option value="all">{t('accountPool.tags.filters.option.all')}</option>
                <option value="true">{t('accountPool.tags.filters.option.guardOn')}</option>
                <option value="false">{t('accountPool.tags.filters.option.guardOff')}</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">{t('accountPool.tags.filters.cutOutBlocked')}</span>
              <select
                name="tagCutOutFilter"
                className="field-select"
                value={cutOutFilter}
                onChange={(event) => {
                  const value = event.target.value as TernaryFilter
                  setCutOutFilter(value)
                  updateQuery({ ...query, allowCutOut: value === 'all' ? undefined : value === 'true' })
                }}
              >
                <option value="all">{t('accountPool.tags.filters.option.all')}</option>
                <option value="true">{t('accountPool.tags.filters.option.allowed')}</option>
                <option value="false">{t('accountPool.tags.filters.option.blocked')}</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label">{t('accountPool.tags.filters.cutInBlocked')}</span>
              <select
                name="tagCutInFilter"
                className="field-select"
                value={cutInFilter}
                onChange={(event) => {
                  const value = event.target.value as TernaryFilter
                  setCutInFilter(value)
                  updateQuery({ ...query, allowCutIn: value === 'all' ? undefined : value === 'true' })
                }}
              >
                <option value="all">{t('accountPool.tags.filters.option.all')}</option>
                <option value="true">{t('accountPool.tags.filters.option.allowed')}</option>
                <option value="false">{t('accountPool.tags.filters.option.blocked')}</option>
              </select>
            </label>
          </div>
        </div>
      </section>

      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-4">
          <div className="flex items-center justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('accountPool.tags.listTitle')}</h2>
              <p className="section-description">{t('accountPool.tags.listDescription')}</p>
            </div>
            {isLoading ? <Spinner className="text-primary" /> : null}
          </div>

          <div className="overflow-hidden rounded-[1.35rem] border border-base-300/80 bg-base-100/72">
            <div className="overflow-x-auto">
              <table className="min-w-[860px] w-full border-collapse">
                <thead>
                  <tr className="border-b border-base-300/80 bg-base-100/86 text-left">
                    <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">{t('accountPool.tags.table.name')}</th>
                    <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">{t('accountPool.tags.table.rule')}</th>
                    <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">{t('accountPool.tags.table.accounts')}</th>
                    <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">{t('accountPool.tags.table.groups')}</th>
                    <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">{t('accountPool.tags.table.updatedAt')}</th>
                    <th className="px-4 py-3" aria-hidden />
                  </tr>
                </thead>
                <tbody>
                  {items.map((tag) => (
                    <tr key={tag.id} className="border-b border-base-300/70 last:border-b-0 hover:bg-base-100/80">
                      <td className="px-4 py-4 font-semibold text-base-content">{tag.name}</td>
                      <td className="px-4 py-4 text-sm text-base-content/70">
                        <div className="flex flex-wrap gap-2">
                          {tag.routingRule.guardEnabled ? (
                            <RuleBadge
                              variant="default"
                              label={t('accountPool.tags.rule.guard', {
                                hours: tag.routingRule.lookbackHours ?? 0,
                                count: tag.routingRule.maxConversations ?? 0,
                              })}
                            />
                          ) : null}
                          {!tag.routingRule.allowCutOut ? (
                            <RuleBadge
                              variant="info"
                              label={t('accountPool.tags.rule.cutOutOff')}
                            />
                          ) : null}
                          {!tag.routingRule.allowCutIn ? (
                            <RuleBadge
                              variant="accent"
                              label={t('accountPool.tags.rule.cutInOff')}
                            />
                          ) : null}
                        </div>
                      </td>
                      <td className="px-4 py-4 text-sm text-base-content/70">{tag.accountCount}</td>
                      <td className="px-4 py-4 text-sm text-base-content/70">{tag.groupCount}</td>
                      <td className="px-4 py-4 text-sm text-base-content/70">{new Date(tag.updatedAt).toLocaleString()}</td>
                      <td className="px-4 py-4 text-right">
                        <div className="flex justify-end gap-2">
                          <Button type="button" variant="ghost" size="sm" onClick={() => openEdit(tag)} disabled={!writesEnabled}>
                            <AppIcon name="pencil-outline" className="h-4 w-4" aria-hidden />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            onClick={() => void deleteTag(tag.id)}
                            disabled={!writesEnabled || tag.accountCount > 0}
                          >
                            <AppIcon name="delete-outline" className="h-4 w-4" aria-hidden />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </section>

      <TagRuleDialog
        open={dialogOpen}
        mode={dialogMode}
        tag={activeTag}
        busy={busy}
        error={dialogError}
        onClose={() => {
          if (busy) return
          setDialogOpen(false)
          setActiveTag(null)
          setDialogError(null)
        }}
        onSubmit={handleSubmit}
        labels={{
          createTitle: t('accountPool.tags.dialog.createTitle'),
          editTitle: t('accountPool.tags.dialog.editTitle'),
          description: t('accountPool.tags.dialog.description'),
          name: t('accountPool.tags.dialog.name'),
          namePlaceholder: t('accountPool.tags.dialog.namePlaceholder'),
          guardEnabled: t('accountPool.tags.dialog.guardEnabled'),
          lookbackHours: t('accountPool.tags.dialog.lookbackHours'),
          maxConversations: t('accountPool.tags.dialog.maxConversations'),
          allowCutOut: t('accountPool.tags.dialog.allowCutOut'),
          allowCutIn: t('accountPool.tags.dialog.allowCutIn'),
          cancel: t('accountPool.tags.dialog.cancel'),
          save: t('accountPool.tags.dialog.save'),
          create: t('accountPool.tags.dialog.createAction'),
          validation: t('accountPool.tags.dialog.validation'),
        }}
      />
    </div>
  )
}
