import { useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import { Alert } from '../../components/ui/alert'
import { Button } from '../../components/ui/button'
import { Input } from '../../components/ui/input'
import { Spinner } from '../../components/ui/spinner'
import { Switch } from '../../components/ui/switch'
import { TagRuleDialog } from '../../components/TagRuleDialog'
import { usePoolTags } from '../../hooks/usePoolTags'
import type { CreateTagPayload, TagSummary, UpdateTagPayload } from '../../lib/api'
import { useTranslation } from '../../i18n'

export default function TagsPage() {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const [hasAccountsOnly, setHasAccountsOnly] = useState(false)
  const [guardEnabledOnly, setGuardEnabledOnly] = useState(false)
  const [cutOutBlockedOnly, setCutOutBlockedOnly] = useState(false)
  const [cutInBlockedOnly, setCutInBlockedOnly] = useState(false)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogMode, setDialogMode] = useState<'create' | 'edit'>('create')
  const [activeTag, setActiveTag] = useState<TagSummary | null>(null)
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const query = useMemo(
    () => ({
      search,
      hasAccounts: hasAccountsOnly ? true : undefined,
      guardEnabled: guardEnabledOnly ? true : undefined,
      allowCutOut: cutOutBlockedOnly ? false : undefined,
      allowCutIn: cutInBlockedOnly ? false : undefined,
    }),
    [cutInBlockedOnly, cutOutBlockedOnly, guardEnabledOnly, hasAccountsOnly, search],
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
                  <Icon icon="mdi:tag-plus-outline" className="mr-2 h-4 w-4" aria-hidden />
                  {t('accountPool.tags.actions.create')}
                </Button>
              ) : null}
            </div>
          </div>

          {error ? (
            <Alert variant="error">
              <Icon icon="mdi:alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{error}</div>
            </Alert>
          ) : null}

          <div className="grid gap-3 lg:grid-cols-[minmax(0,1.1fr)_repeat(4,minmax(10rem,1fr))]">
            <label className="field">
              <span className="field-label">{t('accountPool.tags.filters.search')}</span>
              <Input
                value={search}
                placeholder={t('accountPool.tags.filters.searchPlaceholder')}
                onChange={(event) => {
                  const value = event.target.value
                  setSearch(value)
                  updateQuery({ ...query, search: value })
                }}
              />
            </label>
            <div className="rounded-[1.2rem] border border-base-300/70 bg-base-100/70 px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm text-base-content/70">{t('accountPool.tags.filters.hasAccounts')}</span>
                <Switch checked={hasAccountsOnly} onCheckedChange={(checked) => { setHasAccountsOnly(checked); updateQuery({ ...query, hasAccounts: checked ? true : undefined }) }} />
              </div>
            </div>
            <div className="rounded-[1.2rem] border border-base-300/70 bg-base-100/70 px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm text-base-content/70">{t('accountPool.tags.filters.guardEnabled')}</span>
                <Switch checked={guardEnabledOnly} onCheckedChange={(checked) => { setGuardEnabledOnly(checked); updateQuery({ ...query, guardEnabled: checked ? true : undefined }) }} />
              </div>
            </div>
            <div className="rounded-[1.2rem] border border-base-300/70 bg-base-100/70 px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm text-base-content/70">{t('accountPool.tags.filters.cutOutBlocked')}</span>
                <Switch checked={cutOutBlockedOnly} onCheckedChange={(checked) => { setCutOutBlockedOnly(checked); updateQuery({ ...query, allowCutOut: checked ? false : undefined }) }} />
              </div>
            </div>
            <div className="rounded-[1.2rem] border border-base-300/70 bg-base-100/70 px-4 py-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm text-base-content/70">{t('accountPool.tags.filters.cutInBlocked')}</span>
                <Switch checked={cutInBlockedOnly} onCheckedChange={(checked) => { setCutInBlockedOnly(checked); updateQuery({ ...query, allowCutIn: checked ? false : undefined }) }} />
              </div>
            </div>
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
                          <span>{tag.routingRule.guardEnabled ? t('accountPool.tags.rule.guard', { hours: tag.routingRule.lookbackHours ?? 0, count: tag.routingRule.maxConversations ?? 0 }) : t('accountPool.tags.rule.guardOff')}</span>
                          <span>{tag.routingRule.allowCutOut ? t('accountPool.tags.rule.cutOutOn') : t('accountPool.tags.rule.cutOutOff')}</span>
                          <span>{tag.routingRule.allowCutIn ? t('accountPool.tags.rule.cutInOn') : t('accountPool.tags.rule.cutInOff')}</span>
                        </div>
                      </td>
                      <td className="px-4 py-4 text-sm text-base-content/70">{tag.accountCount}</td>
                      <td className="px-4 py-4 text-sm text-base-content/70">{tag.groupCount}</td>
                      <td className="px-4 py-4 text-sm text-base-content/70">{new Date(tag.updatedAt).toLocaleString()}</td>
                      <td className="px-4 py-4 text-right">
                        <div className="flex justify-end gap-2">
                          <Button type="button" variant="ghost" size="sm" onClick={() => openEdit(tag)} disabled={!writesEnabled}>
                            <Icon icon="mdi:pencil-outline" className="h-4 w-4" aria-hidden />
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            onClick={() => void deleteTag(tag.id)}
                            disabled={!writesEnabled || tag.accountCount > 0}
                          >
                            <Icon icon="mdi:delete-outline" className="h-4 w-4" aria-hidden />
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
