import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Icon } from '@iconify/react'
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
  name,
  placeholder,
  ariaLabel,
  className,
  inputClassName,
}: UpstreamAccountGroupComboboxProps) {
  const rootRef = useRef<HTMLDivElement | null>(null)
  const inputRef = useRef<HTMLInputElement | null>(null)
  const [open, setOpen] = useState(false)
  const [highlightedIndex, setHighlightedIndex] = useState(0)

  const uniqueSuggestions = useMemo(() => normalizeSuggestions(suggestions), [suggestions])
  const normalizedValue = value.trim().toLocaleLowerCase()
  const filteredSuggestions = useMemo(() => {
    if (!normalizedValue) return uniqueSuggestions
    return uniqueSuggestions.filter((suggestion) =>
      suggestion.toLocaleLowerCase().includes(normalizedValue),
    )
  }, [normalizedValue, uniqueSuggestions])

  useEffect(() => {
    setHighlightedIndex(0)
  }, [normalizedValue, open])

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false)
      }
    }

    document.addEventListener('pointerdown', handlePointerDown)
    return () => document.removeEventListener('pointerdown', handlePointerDown)
  }, [])

  const commitValue = (nextValue: string) => {
    onValueChange(nextValue)
    setOpen(false)
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      if (!open) {
        setOpen(true)
        return
      }
      setHighlightedIndex((current) =>
        filteredSuggestions.length === 0 ? 0 : Math.min(current + 1, filteredSuggestions.length - 1),
      )
      return
    }

    if (event.key === 'ArrowUp') {
      event.preventDefault()
      if (!open) {
        setOpen(true)
        return
      }
      setHighlightedIndex((current) => Math.max(current - 1, 0))
      return
    }

    if (event.key === 'Enter') {
      if (!open) return
      event.preventDefault()
      const highlighted = filteredSuggestions[highlightedIndex]
      if (highlighted) {
        commitValue(highlighted)
      } else {
        setOpen(false)
      }
      return
    }

    if (event.key === 'Escape') {
      setOpen(false)
    }
  }

  return (
    <div ref={rootRef} className={cn('relative', className)}>
      <div
        className={cn(
          'flex h-10 w-full items-center overflow-hidden rounded-lg border border-base-300 bg-base-100 shadow-sm',
          'focus-within:ring-2 focus-within:ring-primary focus-within:ring-offset-2 focus-within:ring-offset-base-100',
        )}
      >
        <input
          ref={inputRef}
          type="text"
          name={name}
          value={value}
          autoComplete="off"
          aria-label={ariaLabel}
          aria-autocomplete="list"
          aria-expanded={open}
          role="combobox"
          placeholder={placeholder}
          className={cn(
            'h-full w-full border-0 bg-transparent px-3 text-sm text-base-content placeholder:text-base-content/45 focus:outline-none',
            inputClassName,
          )}
          onFocus={() => setOpen(true)}
          onKeyDown={handleKeyDown}
          onChange={(event) => {
            onValueChange(event.target.value)
            setOpen(true)
          }}
        />
        <button
          type="button"
          aria-label="Toggle group suggestions"
          className={cn(
            'flex h-full w-11 shrink-0 items-center justify-center border-l border-base-300/80 bg-base-100/95 text-base-content/55',
            'hover:bg-base-200/70 hover:text-base-content/85',
          )}
          onClick={() => {
            setOpen((current) => !current)
            inputRef.current?.focus()
          }}
        >
          <Icon
            icon={open ? 'mdi:chevron-up' : 'mdi:chevron-down'}
            className="h-4 w-4"
            aria-hidden
          />
        </button>
      </div>

      {open && filteredSuggestions.length > 0 ? (
        <div className="absolute left-0 right-0 top-[calc(100%+0.4rem)] z-30 rounded-xl border border-base-300/90 bg-base-100/98 p-1.5 shadow-2xl backdrop-blur">
          <ul className="max-h-56 overflow-y-auto" role="listbox">
            {filteredSuggestions.map((suggestion, index) => {
              const selected = suggestion === value
              const highlighted = highlightedIndex === index
              return (
                <li key={suggestion}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={selected}
                    className={cn(
                      'flex w-full items-center rounded-lg px-3 py-2 text-left text-sm text-base-content/82',
                      highlighted && 'bg-primary/16 text-base-content',
                      selected && 'font-semibold text-primary',
                    )}
                    onMouseEnter={() => setHighlightedIndex(index)}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => commitValue(suggestion)}
                  >
                    <span className="truncate">{suggestion}</span>
                    {selected ? (
                      <Icon icon="mdi:check" className="ml-auto h-4 w-4 shrink-0 text-primary" aria-hidden />
                    ) : null}
                  </button>
                </li>
              )
            })}
          </ul>
        </div>
      ) : null}
    </div>
  )
}
