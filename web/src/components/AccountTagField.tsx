import { useMemo, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Button } from './ui/button'
import { AccountTagContextChip } from './AccountTagContextChip'
import { Command, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from './ui/command'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import { TagRuleDialog } from './TagRuleDialog'
import type { CreateTagPayload, TagDetail, TagSummary, UpdateTagPayload } from '../lib/api'
import { cn } from '../lib/utils'

interface AccountTagFieldLabels {
  label: string
  add: string
  empty: string
  searchPlaceholder: string
  searchEmpty: string
  createInline: (value: string) => string
  selectedFromCurrentPage: string
  remove: string
  deleteAndRemove: string
  edit: string
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
  priorityTier: string
  priorityPrimary: string
  priorityNormal: string
  priorityFallback: string
  fastModeRewriteMode: string
  fastModeKeepOriginal: string
  fastModeFillMissing: string
  fastModeForceAdd: string
  fastModeForceRemove: string
  concurrencyLimit?: string
  concurrencyHint?: string
  currentValue?: string
  unlimited?: string
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
  const [createNameSeed, setCreateNameSeed] = useState('')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [dialogMode, setDialogMode] = useState<'create' | 'edit'>('create')
  const [dialogError, setDialogError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const selectedTags = useMemo(() => {
    const tagMap = new Map(tags.map((tag) => [tag.id, tag]))
    return selectedTagIds
      .map((tagId) => tagMap.get(tagId))
      .filter((tag): tag is TagSummary => tag != null)
  }, [selectedTagIds, tags])
  const filteredTags = useMemo(() => {
    const keyword = search.trim().toLowerCase()
    if (!keyword) return tags
    return tags.filter((tag) => tag.name.toLowerCase().includes(keyword))
  }, [search, tags])
  const selectedSet = useMemo(() => new Set(selectedTagIds), [selectedTagIds])
  const pageCreatedSet = useMemo(() => new Set(pageCreatedTagIds), [pageCreatedTagIds])
  const trimmedSearch = search.trim()

  const toggleTag = (tagId: number) => {
    if (selectedSet.has(tagId)) {
      onChange(selectedTagIds.filter((value) => value !== tagId))
      return
    }
    onChange([...selectedTagIds, tagId])
  }

  const openCreateDialog = (nameSeed = '') => {
    setPickerOpen(false)
    setSearch('')
    setDialogMode('create')
    setEditingTag(null)
    setCreateNameSeed(nameSeed)
    setDialogError(null)
    setDialogOpen(true)
  }

  const openEditDialog = (tag: TagSummary) => {
    setPickerOpen(false)
    setDialogMode('edit')
    setEditingTag(tag)
    setCreateNameSeed('')
    setDialogError(null)
    setDialogOpen(true)
  }

  const closeDialog = () => {
    if (busy) return
    setEditingTag(null)
    setCreateNameSeed('')
    setDialogError(null)
    setDialogOpen(false)
  }

  const handleSubmit = async (payload: CreateTagPayload | UpdateTagPayload) => {
    setBusy(true)
    setDialogError(null)
    try {
      if (dialogMode === 'create') {
        const created = await onCreateTag(payload as CreateTagPayload)
        onChange(selectedSet.has(created.id) ? selectedTagIds : [...selectedTagIds, created.id])
        setSearch('')
      } else if (editingTag) {
        await onUpdateTag(editingTag.id, payload)
      }
      setEditingTag(null)
      setCreateNameSeed('')
      setDialogError(null)
      setDialogOpen(false)
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

  const handlePickerOpenChange = (nextOpen: boolean) => {
    if (!writesEnabled) {
      setPickerOpen(false)
      return
    }
    setPickerOpen(nextOpen)
    if (!nextOpen) setSearch('')
  }

  return (
    <div className="field gap-3">
      <span className="field-label">{labels.label}</span>
      <div
        className={cn(
          'flex min-h-12 items-center gap-2 rounded-[1.2rem] border border-base-300/80 bg-base-100/55 px-3 py-2 shadow-sm transition-colors',
          'focus-within:outline-none focus-within:ring-2 focus-within:ring-primary focus-within:ring-offset-2 focus-within:ring-offset-base-100',
          pickerOpen && 'border-primary/35 bg-base-100/80',
        )}
      >
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          {selectedTags.length === 0 ? <span className="pl-0.5 text-sm text-base-content/60">{labels.empty}</span> : null}
          {selectedTags.map((tag) => {
            const currentPageCreated = pageCreatedSet.has(tag.id)
            return (
              <AccountTagContextChip
                key={tag.id}
                name={tag.name}
                currentPageCreated={currentPageCreated}
                disabled={!writesEnabled}
                labels={{
                  selectedFromCurrentPage: labels.selectedFromCurrentPage,
                  remove: labels.remove,
                  deleteAndRemove: labels.deleteAndRemove,
                  edit: labels.edit,
                }}
                onRemove={() => handleRemove(tag)}
                onEdit={() => openEditDialog(tag)}
              />
            )
          })}
        </div>

        <Popover open={writesEnabled ? pickerOpen : false} onOpenChange={handlePickerOpenChange}>
          <PopoverTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label={labels.add}
              title={labels.add}
              disabled={!writesEnabled}
              className={cn(
                'h-8 w-8 shrink-0 rounded-full border border-base-300/80 bg-base-100/90 text-base-content/65 shadow-none',
                'hover:bg-base-200 hover:text-base-content',
                'data-[state=open]:border-primary/35 data-[state=open]:bg-primary/10 data-[state=open]:text-primary',
              )}
            >
              <AppIcon name="tag-plus-outline" className="h-4 w-4" aria-hidden />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" className="w-80 max-w-[calc(100vw-2rem)] p-0">
            <Command shouldFilter={false}>
              <CommandInput value={search} placeholder={labels.searchPlaceholder} onValueChange={setSearch} />
              <CommandList className="max-h-72">
                {filteredTags.length > 0 ? (
                  <CommandGroup>
                    {filteredTags.map((tag) => {
                      const selected = selectedSet.has(tag.id)
                      return (
                        <CommandItem key={tag.id} value={tag.name} className="gap-2 px-3 py-2.5" onSelect={() => toggleTag(tag.id)}>
                          <AppIcon
                            name="check"
                            className={cn('h-4 w-4 shrink-0 text-primary transition-opacity', selected ? 'opacity-100' : 'opacity-0')}
                            aria-hidden
                          />
                          <span className={cn('flex-1 truncate', selected && 'font-medium')}>{tag.name}</span>
                        </CommandItem>
                      )
                    })}
                  </CommandGroup>
                ) : null}
                {writesEnabled ? (
                  <>
                    {filteredTags.length > 0 ? <CommandSeparator /> : null}
                    <CommandGroup>
                      <CommandItem
                        value={`__create__:${trimmedSearch || labels.add}`}
                        className="gap-2 px-3 py-2.5"
                        onSelect={() => openCreateDialog(trimmedSearch)}
                      >
                        <AppIcon name="plus-circle-outline" className="h-4 w-4 shrink-0 text-primary" aria-hidden />
                        <span className="truncate">{labels.createInline(trimmedSearch)}</span>
                      </CommandItem>
                    </CommandGroup>
                  </>
                ) : filteredTags.length === 0 ? (
                  <div className="px-3 py-6 text-sm text-base-content/60">{labels.searchEmpty}</div>
                ) : null}
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
      </div>

      <TagRuleDialog
        open={dialogOpen}
        mode={editingTag ? 'edit' : 'create'}
        tag={editingTag}
        draftName={dialogMode === 'create' ? createNameSeed : undefined}
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
          priorityTier: labels.priorityTier,
          priorityPrimary: labels.priorityPrimary,
          priorityNormal: labels.priorityNormal,
          priorityFallback: labels.priorityFallback,
          fastModeRewriteMode: labels.fastModeRewriteMode,
          fastModeKeepOriginal: labels.fastModeKeepOriginal,
          fastModeFillMissing: labels.fastModeFillMissing,
          fastModeForceAdd: labels.fastModeForceAdd,
          fastModeForceRemove: labels.fastModeForceRemove,
          concurrencyLimit: labels.concurrencyLimit,
          concurrencyHint: labels.concurrencyHint,
          currentValue: labels.currentValue,
          unlimited: labels.unlimited,
          cancel: labels.cancel,
          save: labels.save,
          create: labels.createAction,
          validation: labels.validation,
        }}
      />
    </div>
  )
}
