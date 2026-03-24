import { useMemo, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Button } from './ui/button'
import { formControlSizeVariants, type FormControlSize } from './ui/form-control'
import { Command, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from './ui/command'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import type { TagSummary } from '../lib/api'
import { cn } from '../lib/utils'

interface AccountTagFilterComboboxProps {
  tags: TagSummary[]
  value: number[]
  onValueChange: (value: number[]) => void
  disabled?: boolean
  prioritizedTagIds?: number[]
  disabledTagIds?: number[]
  placeholder?: string
  searchPlaceholder?: string
  emptyLabel?: string
  clearLabel?: string
  ariaLabel?: string
  className?: string
  triggerClassName?: string
  size?: FormControlSize
}

function normalizeTags(tags: TagSummary[]) {
  const deduped = new Map<number, TagSummary>()
  for (const tag of tags) {
    if (!deduped.has(tag.id)) {
      deduped.set(tag.id, tag)
    }
  }
  return Array.from(deduped.values()).sort((left, right) => left.name.localeCompare(right.name))
}

function buildTriggerLabel(selectedTags: TagSummary[], placeholder: string) {
  if (selectedTags.length === 0) return placeholder
  if (selectedTags.length === 1) return selectedTags[0].name
  if (selectedTags.length === 2) return `${selectedTags[0].name}, ${selectedTags[1].name}`
  return `${selectedTags[0].name}, ${selectedTags[1].name} +${selectedTags.length - 2}`
}

export function AccountTagFilterCombobox({
  tags,
  value,
  onValueChange,
  disabled = false,
  prioritizedTagIds,
  disabledTagIds,
  placeholder = 'All tags',
  searchPlaceholder = 'Search tags...',
  emptyLabel = 'No matching tags.',
  clearLabel = 'Clear tag filters',
  ariaLabel,
  className,
  triggerClassName,
  size = 'default',
}: AccountTagFilterComboboxProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')

  const availableTags = useMemo(() => normalizeTags(tags), [tags])
  const selectedTagIdSet = useMemo(() => new Set(value), [value])
  const prioritizedTagIdSet = useMemo(
    () => new Set(prioritizedTagIds ?? []),
    [prioritizedTagIds],
  )
  const disabledTagIdSet = useMemo(
    () => new Set(disabledTagIds ?? []),
    [disabledTagIds],
  )
  const selectedTags = useMemo(
    () => availableTags.filter((tag) => selectedTagIdSet.has(tag.id)),
    [availableTags, selectedTagIdSet],
  )
  const filteredTags = useMemo(() => {
    const keyword = query.trim().toLocaleLowerCase()
    const visibleTags = keyword
      ? availableTags.filter((tag) => tag.name.toLocaleLowerCase().includes(keyword))
      : availableTags
    return [...visibleTags].sort((left, right) => {
      const leftPrioritized = prioritizedTagIdSet.has(left.id)
      const rightPrioritized = prioritizedTagIdSet.has(right.id)
      if (leftPrioritized !== rightPrioritized) {
        return leftPrioritized ? -1 : 1
      }
      const leftDisabled = disabledTagIdSet.has(left.id)
      const rightDisabled = disabledTagIdSet.has(right.id)
      if (leftDisabled !== rightDisabled) {
        return leftDisabled ? 1 : -1
      }
      return left.name.localeCompare(right.name)
    })
  }, [availableTags, disabledTagIdSet, prioritizedTagIdSet, query])

  const toggleTag = (tagId: number) => {
    if (disabledTagIdSet.has(tagId)) return
    const next = selectedTagIdSet.has(tagId)
      ? value.filter((currentId) => currentId !== tagId)
      : [...value, tagId]
    onValueChange(next)
  }

  const triggerLabel = buildTriggerLabel(selectedTags, placeholder)
  const triggerTitle = selectedTags.length > 0 ? selectedTags.map((tag) => tag.name).join(', ') : undefined

  return (
    <div className={className}>
      <Popover
        open={disabled ? false : open}
        onOpenChange={(nextOpen) => {
          if (disabled) {
            setOpen(false)
            return
          }
          setOpen(nextOpen)
          if (!nextOpen) {
            setQuery('')
          }
        }}
      >
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="outline"
            role="combobox"
            aria-expanded={open}
            aria-label={ariaLabel}
            disabled={disabled}
            className={cn(
              'w-full justify-between bg-base-100 text-left font-normal hover:bg-base-100',
              'border-base-300 text-base-content shadow-sm',
              formControlSizeVariants({ size }),
              selectedTags.length === 0 && 'text-base-content/45',
              triggerClassName,
            )}
            title={triggerTitle}
          >
            <span className="flex min-w-0 items-center gap-2">
              <AppIcon name="tag-outline" className="h-4 w-4 shrink-0 text-base-content/55" aria-hidden />
              <span className="truncate">{triggerLabel}</span>
            </span>
            <AppIcon name="chevron-down" className="ml-2 h-4 w-4 shrink-0 text-base-content/45" aria-hidden />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-[var(--radix-popover-trigger-width)] p-0">
          <Command shouldFilter={false}>
            <CommandInput
              value={query}
              placeholder={searchPlaceholder}
              onValueChange={setQuery}
            />
            <CommandList>
              {selectedTags.length > 0 ? (
                <>
                  <CommandGroup>
                    <CommandItem onSelect={() => onValueChange([])}>
                      <AppIcon name="close" className="mr-2 h-4 w-4 text-base-content/55" aria-hidden />
                      <span className="truncate">{clearLabel}</span>
                    </CommandItem>
                  </CommandGroup>
                  <CommandSeparator />
                </>
              ) : null}
              {filteredTags.length === 0 ? (
                <div className="py-6 text-center text-sm text-base-content/60">{emptyLabel}</div>
              ) : (
                <CommandGroup>
                  {filteredTags.map((tag) => {
                    const selected = selectedTagIdSet.has(tag.id)
                    const tagDisabled = disabledTagIdSet.has(tag.id)
                    return (
                      <CommandItem
                        key={tag.id}
                        value={tag.name}
                        disabled={tagDisabled}
                        onSelect={() => toggleTag(tag.id)}
                        className={cn(tagDisabled && 'text-base-content/40')}
                      >
                        <AppIcon
                          name="check"
                          className={cn(
                            'mr-2 h-4 w-4 text-primary transition-opacity',
                            selected ? 'opacity-100' : 'opacity-0',
                          )}
                          aria-hidden
                        />
                        <span className="truncate">{tag.name}</span>
                      </CommandItem>
                    )
                  })}
                </CommandGroup>
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  )
}
