import type { ReactNode } from 'react'
import { FloatingFieldError } from './floating-field-error'
import { FloatingFieldBubble } from './floating-field-bubble'
import type { BubbleVariant } from './bubble'
import { cn } from '../../lib/utils'

interface FormFieldFeedbackProps {
  label: ReactNode
  message?: string | null
  variant?: BubbleVariant
  className?: string
  labelClassName?: string
  messageClassName?: string
}

export function FormFieldFeedback({
  label,
  message,
  variant = 'error',
  className,
  labelClassName,
  messageClassName,
}: FormFieldFeedbackProps) {
  const feedback = !message ? null : variant === 'error'
    ? (
      <FloatingFieldError
        placement="label-inline"
        message={message}
        className={messageClassName}
      />
      )
    : (
      <FloatingFieldBubble
        placement="label-inline"
        message={message}
        variant={variant}
        className={messageClassName}
      />
      )

  return (
    <div
      className={cn(
        'flex flex-wrap items-start gap-2 md:min-h-7 md:flex-nowrap md:items-center md:justify-between',
        className,
      )}
    >
      <span className={cn('field-label', labelClassName)}>{label}</span>
      {feedback}
    </div>
  )
}
