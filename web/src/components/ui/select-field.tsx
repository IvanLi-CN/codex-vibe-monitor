import type { ComponentPropsWithoutRef } from 'react'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from './select'
import { cn } from '../../lib/utils'

const EMPTY_OPTION_SENTINEL = '__cvm_selectfield_empty__'

export type SelectFieldOption = {
  value: string
  label: string
  disabled?: boolean
}

export interface SelectFieldProps {
  options: readonly SelectFieldOption[]
  value: string
  onValueChange: (value: string) => void
  label?: string
  name?: string
  placeholder?: string
  size?: 'default' | 'sm'
  disabled?: boolean
  className?: string
  triggerClassName?: string
  id?: string
  'data-testid'?: string
  'aria-label'?: string
}

function toInternalValue(value: string) {
  return value === '' ? EMPTY_OPTION_SENTINEL : value
}

function fromInternalValue(value: string) {
  return value === EMPTY_OPTION_SENTINEL ? '' : value
}

export function SelectField({
  options,
  value,
  onValueChange,
  label,
  name,
  placeholder,
  size = 'default',
  disabled = false,
  className,
  triggerClassName,
  id,
  'data-testid': dataTestId,
  'aria-label': ariaLabel,
}: SelectFieldProps) {
  const wrapperProps = {
    className: label ? cn('field', className) : className,
  } satisfies ComponentPropsWithoutRef<'div'>
  const selectValue = options.some((option) => option.value === value)
    ? toInternalValue(value)
    : undefined
  const triggerSizeClass = size === 'sm' ? 'h-8 text-sm' : 'h-9 text-sm'

  const content = (
    <>
      {name ? (
        <input type="hidden" name={name} value={value} disabled={disabled} />
      ) : null}
      <Select
        value={selectValue}
        onValueChange={(nextValue) => onValueChange(fromInternalValue(nextValue))}
        disabled={disabled}
      >
        <SelectTrigger
          id={id}
          data-testid={dataTestId}
          aria-label={ariaLabel ?? label}
          className={cn('w-full', triggerSizeClass, triggerClassName)}
        >
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem
              key={`${option.value || '__empty'}:${option.label}`}
              value={toInternalValue(option.value)}
              disabled={option.disabled}
            >
              {option.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </>
  )

  if (!label) {
    return <div {...wrapperProps}>{content}</div>
  }

  return (
    <label className={wrapperProps.className}>
      <span className="field-label">{label}</span>
      {content}
    </label>
  )
}
