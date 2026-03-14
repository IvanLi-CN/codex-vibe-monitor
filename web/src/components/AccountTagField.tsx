import { useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import { Button } from './ui/button'
import { AccountTagContextChip } from './AccountTagContextChip'
import { Input } from './ui/input'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import { TagRuleDialog } from './TagRuleDialog'
import type { CreateTagPayload, TagDetail, TagSummary, UpdateTagPayload } from '../lib/api'
import { cn } from '../lib/utils'

interface AccountTagFieldLabels {
  label: string
  add: string
  empty: string
  searchPlaceholder: string
  createInline: (value: string) => string
  selectedFromCurrentPage: string
  remove: string
  deleteAndRemove: string
  edit: string
  hoverHint: string
  createTitle: string
  editTitle: string
  dialogDescription: string
  name: string
  namePlaceholder: string
  guardEnabled: string
  lookbackHours: string
  maxConversations: string
  allowCutOut: string
  allowCutIn: string
  cancel: string
  save: string
  createAction: string
  validation: string
}

interface AccountTagFieldProps {
  tags: TagSummary[]
  selectedTagIds: number[]
  writesEnabled: boolean
  pageCreatedTagIds?: number[]
  labels: AccountTagFieldLabels
  onChange: (tagIds: number[]) => void
  onCreateTag: (payload: CreateTagPayload) => Promise<TagDetail>
  onUpdateTag: (tagId: number, payload: UpdateTagPayload) => Promise<TagDetail>
  onDeleteTag: (tagId: number) => Promise<void>
}

export function AccountTagField({
  tags,
  selectedTagIds,
  writesEnabled,
  pageCreatedTagIds = [],
  labels,
  onChange,
  onCreateTag,
  onUpdateTag,
  onDeleteTag,
}: AccountTagFieldProps) {
  const [search, setSearch] = useState('')
  const [pickerOpen, setPickerOpen] = useState(false)
  const [editingTag, setEditingTag] = useState<TagSummary | null>(null)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogMode, setDialogMode] = useState<'create' | 'edit'>('create')
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const selectedTags = useMemo(
    () => tags.filter((tag) => selectedTagIds.includes(tag.id)),
    [selectedTagIds, tags],
  )
  const filteredTags = useMemo(() => {
    const keyword = search.trim().toLowerCase()
    if (!keyword) return tags
    return tags.filter((tag) => tag.name.toLowerCase().includes(keyword))
  }, [search, tags])
  const selectedSet = useMemo(() => new Set(selectedTagIds), [selectedTagIds])
  const pageCreatedSet = useMemo(() => new Set(pageCreatedTagIds), [pageCreatedTagIds])

  const toggleTag = (tagId: number) => {
    if (selectedSet.has(tagId)) {
      onChange(selectedTagIds.filter((value) => value !== tagId))
      return
    }
    onChange([...selectedTagIds, tagId])
  }

  const openCreateDialog = () => {
    setDialogMode('create')
    setEditingTag(null)
    setDialogError(null)
    setDialogOpen(true)
  }

  const openEditDialog = (tag: TagSummary) => {
    setDialogMode('edit')
    setEditingTag(tag)
    setDialogError(null)
    setDialogOpen(true)
  }

  const closeDialog = () => {
    if (busy) return
    setEditingTag(null)
    setDialogError(null)
    setDialogOpen(false)
  }

  const handleSubmit = async (payload: CreateTagPayload | UpdateTagPayload) => {
    setBusy(true)
    setDialogError(null)
    try {
      if (dialogMode === 'create') {
        const created = await onCreateTag(payload as CreateTagPayload)
        onChange([...selectedTagIds, created.id])
        setSearch('')
      } else if (editingTag) {
        await onUpdateTag(editingTag.id, payload)
      }
      closeDialog()
    } catch (err) {
      setDialogError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const handleRemove = async (tag: TagSummary) => {
    if (pageCreatedSet.has(tag.id)) {
      try {
        await onDeleteTag(tag.id)
      } catch {
        // Fall back to just unlinking if the tag is already referenced elsewhere.
      }
    }
    onChange(selectedTagIds.filter((value) => value !== tag.id))
  }

  return (
    <div className="field gap-3">
      <div className="flex items-center justify-between gap-3">
        <span className="field-label">{labels.label}</span>
        <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
          <PopoverTrigger asChild>
            <Button type="button" variant="outline" size="sm" disabled={!writesEnabled}>
              <Icon icon="mdi:tag-plus-outline" className="mr-2 h-4 w-4" aria-hidden />
              {labels.add}
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-[22rem] space-y-3 p-3">
            <Input
              name="tagSearch"
              value={search}
              placeholder={labels.searchPlaceholder}
              onChange={(event) => setSearch(event.target.value)}
            />
            <div className="max-h-64 space-y-1 overflow-y-auto">
              {filteredTags.map((tag) => {
                const selected = selectedSet.has(tag.id)
                return (
                  <button
                    key={tag.id}
                    type="button"
                    className={cn(
                      'flex w-full items-center justify-between rounded-xl px-3 py-2 text-left text-sm transition-colors',
                      selected ? 'bg-primary/10 text-primary' : 'hover:bg-base-200/70',
                    )}
                    onClick={() => toggleTag(tag.id)}
                  >
                    <span className="truncate">{tag.name}</span>
                    {selected ? <Icon icon="mdi:check" className="h-4 w-4" aria-hidden /> : null}
                  </button>
                )
              })}
              {filteredTags.length === 0 ? <p className="px-2 py-3 text-sm text-base-content/60">{labels.empty}</p> : null}
            </div>
            {writesEnabled ? (
              <Button
                type="button"
                variant="secondary"
                className="w-full"
                onClick={() => {
                  setPickerOpen(false)
                  openCreateDialog()
                }}
              >
                {labels.createInline(search.trim())}
              </Button>
            ) : null}
          </PopoverContent>
        </Popover>
      </div>

      <div className="flex flex-wrap gap-2 rounded-[1.2rem] border border-dashed border-base-300/80 bg-base-100/45 p-3">
        {selectedTags.length === 0 ? <p className="text-sm text-base-content/60">{labels.empty}</p> : null}
        {selectedTags.map((tag) => {
          const currentPageCreated = pageCreatedSet.has(tag.id)
          return (
            <AccountTagContextChip
              key={tag.id}
              name={tag.name}
              currentPageCreated={currentPageCreated}
              labels={{
                selectedFromCurrentPage: labels.selectedFromCurrentPage,
                remove: labels.remove,
                deleteAndRemove: labels.deleteAndRemove,
                edit: labels.edit,
                hoverHint: labels.hoverHint,
              }}
              onRemove={() => handleRemove(tag)}
              onEdit={() => openEditDialog(tag)}
            />
          )
        })}
      </div>

      <TagRuleDialog
        open={dialogOpen}
        mode={editingTag ? 'edit' : 'create'}
        tag={editingTag}
        busy={busy}
        error={dialogError}
        onClose={closeDialog}
        onSubmit={handleSubmit}
        labels={{
          createTitle: labels.createTitle,
          editTitle: labels.editTitle,
          description: labels.dialogDescription,
          name: labels.name,
          namePlaceholder: labels.namePlaceholder,
          guardEnabled: labels.guardEnabled,
          lookbackHours: labels.lookbackHours,
          maxConversations: labels.maxConversations,
          allowCutOut: labels.allowCutOut,
          allowCutIn: labels.allowCutIn,
          cancel: labels.cancel,
          save: labels.save,
          create: labels.createAction,
          validation: labels.validation,
        }}
      />
    </div>
  )
}
