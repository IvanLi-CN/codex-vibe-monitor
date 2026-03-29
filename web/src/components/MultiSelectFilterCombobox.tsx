import { useMemo, useState } from 'react'
import { AppIcon, type AppIconName } from './AppIcon'
import { Button } from './ui/button'
import { formControlSizeVariants, type FormControlSize } from './ui/form-control'
import { Command, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from './ui/command'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import { cn } from '../lib/utils'

export interface MultiSelectFilterOption {
  value: string
  label: string
  disabled?: boolean
}

interface MultiSelectFilterComboboxProps {
  options: MultiSelectFilterOption[]
  value: string[]
  onValueChange: (value: string[]) => void
  disabled?: boolean
  placeholder?: string
  searchPlaceholder?: string
  emptyLabel?: string
  clearLabel?: string
  ariaLabel?: string
  className?: string
  triggerClassName?: string
  size?: FormControlSize
  iconName?: AppIconName
}

function normalizeOptions(options: MultiSelectFilterOption[]) {
  const deduped = new Map<string, MultiSelectFilterOption>()
  for (const option of options) {
    const normalizedValue = option.value.trim()
    if (!normalizedValue || deduped.has(normalizedValue)) continue
    deduped.set(normalizedValue, {
      ...option,
      value: normalizedValue,
    })
  }
  return Array.from(deduped.values())
}

function buildTriggerLabel(selectedOptions: MultiSelectFilterOption[], placeholder: string) {
  if (selectedOptions.length === 0) return placeholder
  if (selectedOptions.length === 1) return selectedOptions[0].label
  if (selectedOptions.length === 2) {
    return `${selectedOptions[0].label}, ${selectedOptions[1].label}`
  }
  return `${selectedOptions[0].label}, ${selectedOptions[1].label} +${selectedOptions.length - 2}`
}

export function MultiSelectFilterCombobox({
  options,
  value,
  onValueChange,
  disabled = false,
  placeholder = 'All',
  searchPlaceholder = 'Search...',
  emptyLabel = 'No matching options.',
  clearLabel = 'Clear filters',
  ariaLabel,
  className,
  triggerClassName,
  size = 'default',
  iconName,
}: MultiSelectFilterComboboxProps) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')

  const availableOptions = useMemo(() => normalizeOptions(options), [options])
  const selectedValueSet = useMemo(() => new Set(value), [value])
  const selectedOptions = useMemo(
    () => availableOptions.filter((option) => selectedValueSet.has(option.value)),
    [availableOptions, selectedValueSet],
  )
  const filteredOptions = useMemo(() => {
    const keyword = query.trim().toLocaleLowerCase()
    if (!keyword) return availableOptions
    return availableOptions.filter((option) => option.label.toLocaleLowerCase().includes(keyword))
  }, [availableOptions, query])

  const toggleOption = (nextValue: string) => {
    const next = selectedValueSet.has(nextValue)
      ? value.filter((currentValue) => currentValue !== nextValue)
      : [...value, nextValue]
    onValueChange(next)
  }

  const triggerLabel = buildTriggerLabel(selectedOptions, placeholder)
  const triggerTitle = selectedOptions.length > 0
    ? selectedOptions.map((option) => option.label).join(', ')
    : undefined

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
              selectedOptions.length === 0 && 'text-base-content/45',
              triggerClassName,
            )}
            title={triggerTitle}
          >
            <span className="flex min-w-0 items-center gap-2">
              {iconName ? (
                <AppIcon name={iconName} className="h-4 w-4 shrink-0 text-base-content/55" aria-hidden />
              ) : null}
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
              {selectedOptions.length > 0 ? (
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
              {filteredOptions.length === 0 ? (
                <div className="py-6 text-center text-sm text-base-content/60">{emptyLabel}</div>
              ) : (
                <CommandGroup>
                  {filteredOptions.map((option) => {
                    const selected = selectedValueSet.has(option.value)
                    return (
                      <CommandItem
                        key={option.value}
                        value={option.label}
                        disabled={option.disabled}
                        onSelect={() => toggleOption(option.value)}
                        className={cn(option.disabled && 'text-base-content/40')}
                      >
                        <AppIcon
                          name="check"
                          className={cn(
                            'mr-2 h-4 w-4 text-primary transition-opacity',
                            selected ? 'opacity-100' : 'opacity-0',
                          )}
                          aria-hidden
                        />
                        <span className="truncate">{option.label}</span>
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
