import { useId, useMemo } from 'react'
import { Icon } from '@iconify/react'
import { Input } from './ui/input'
import { cn } from '../lib/utils'

interface UpstreamAccountGroupComboboxProps {
  value: string
  onValueChange: (value: string) => void
  suggestions: string[]
  name?: string
  placeholder?: string
  ariaLabel?: string
  className?: string
  inputClassName?: string
}

export function UpstreamAccountGroupCombobox({
  value,
  onValueChange,
  suggestions,
  name,
  placeholder,
  ariaLabel,
  className,
  inputClassName,
}: UpstreamAccountGroupComboboxProps) {
  const listId = useId()
  const uniqueSuggestions = useMemo(() => {
    const deduped = new Set<string>()
    for (const suggestion of suggestions) {
      const normalized = suggestion.trim()
      if (normalized) {
        deduped.add(normalized)
      }
    }
    return Array.from(deduped)
  }, [suggestions])

  return (
    <div className={cn('relative', className)}>
      <Input
        list={listId}
        name={name}
        value={value}
        autoComplete="off"
        aria-label={ariaLabel}
        placeholder={placeholder}
        className={cn('pr-10', inputClassName)}
        onChange={(event) => onValueChange(event.target.value)}
      />
      <Icon
        icon="mdi:chevron-down"
        className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-base-content/45"
        aria-hidden
      />
      <datalist id={listId}>
        {uniqueSuggestions.map((suggestion) => (
          <option key={suggestion} value={suggestion} />
        ))}
      </datalist>
    </div>
  )
}
