import { useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import { Button } from './ui/button'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from './ui/command'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import { cn } from '../lib/utils'

interface UpstreamAccountGroupComboboxProps {
  value: string
  onValueChange: (value: string) => void
  suggestions: string[]
  disabled?: boolean
  name?: string
  placeholder?: string
  searchPlaceholder?: string
  emptyLabel?: string
  createLabel?: (value: string) => string
  ariaLabel?: string
  className?: string
  triggerClassName?: string
}

function normalizeSuggestions(suggestions: string[]) {
  const deduped = new Set<string>()
  for (const suggestion of suggestions) {
    const normalized = suggestion.trim()
    if (normalized) {
      deduped.add(normalized)
    }
  }
  return Array.from(deduped)
}

export function UpstreamAccountGroupCombobox({
  value,
  onValueChange,
  suggestions,
  disabled = false,
  name,
  placeholder,
  searchPlaceholder,
  emptyLabel = 'No groups found.',
  createLabel = (nextValue) => `Use "${nextValue}"`,
  ariaLabel,
  className,
  triggerClassName,
}: UpstreamAccountGroupComboboxProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const uniqueSuggestions = useMemo(() => normalizeSuggestions(suggestions), [suggestions])
  const trimmedValue = value.trim()
  const trimmedQuery = query.trim()
  const showCreateOption =
    trimmedQuery.length > 0
    && !uniqueSuggestions.some(
      (suggestion) => suggestion.toLocaleLowerCase() === trimmedQuery.toLocaleLowerCase(),
    )

  const commitValue = (nextValue: string) => {
    onValueChange(nextValue)
    setQuery('')
    setOpen(false)
  }

  return (
    <div className={className}>
      <input type="hidden" name={name} value={value} />
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
              'h-10 w-full justify-between rounded-lg bg-base-100 px-3 text-left font-normal hover:bg-base-100',
              'border-base-300 text-base-content shadow-sm',
              !trimmedValue && 'text-base-content/45',
              triggerClassName,
            )}
          >
            <span className="truncate">{trimmedValue || placeholder}</span>
            <Icon icon="mdi:chevron-down" className="ml-2 h-4 w-4 shrink-0 text-base-content/45" aria-hidden />
          </Button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-[var(--radix-popover-trigger-width)] p-0">
          <Command shouldFilter>
            <CommandInput
              value={query}
              placeholder={searchPlaceholder}
              onValueChange={setQuery}
            />
            <CommandList>
              <CommandEmpty>{emptyLabel}</CommandEmpty>
              <CommandGroup>
                {showCreateOption ? (
                  <>
                    <CommandItem value={trimmedQuery} onSelect={() => commitValue(trimmedQuery)}>
                      <Icon icon="mdi:plus-circle-outline" className="mr-2 h-4 w-4 text-primary" aria-hidden />
                      <span className="truncate">{createLabel(trimmedQuery)}</span>
                    </CommandItem>
                    <CommandSeparator />
                  </>
                ) : null}
                {uniqueSuggestions.map((suggestion) => (
                  <CommandItem key={suggestion} value={suggestion} onSelect={() => commitValue(suggestion)}>
                    <Icon
                      icon="mdi:check"
                      className={cn(
                        'mr-2 h-4 w-4 text-primary transition-opacity',
                        suggestion === trimmedValue ? 'opacity-100' : 'opacity-0',
                      )}
                      aria-hidden
                    />
                    <span className="truncate">{suggestion}</span>
                  </CommandItem>
                ))}
              </CommandGroup>
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    </div>
  )
}
