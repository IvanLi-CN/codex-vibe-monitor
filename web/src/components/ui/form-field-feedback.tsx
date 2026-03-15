import type { ReactNode } from 'react'
import { FloatingFieldError } from './floating-field-error'
import { cn } from '../../lib/utils'

interface FormFieldFeedbackProps {
  label: ReactNode
  message?: string | null
  className?: string
  labelClassName?: string
  messageClassName?: string
}

export function FormFieldFeedback({
  label,
  message,
  className,
  labelClassName,
  messageClassName,
}: FormFieldFeedbackProps) {
  return (
    <div
      className={cn(
        'flex flex-wrap items-start gap-2 md:min-h-7 md:flex-nowrap md:items-center md:justify-between',
        className,
      )}
    >
      <span className={cn('field-label', labelClassName)}>{label}</span>
      {message ? (
        <FloatingFieldError
          placement="label-inline"
          message={message}
          className={messageClassName}
        />
      ) : null}
    </div>
  )
}
