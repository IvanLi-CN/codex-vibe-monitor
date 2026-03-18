import { FloatingFieldBubble, type FloatingFieldBubblePlacement } from './floating-field-bubble'

interface FloatingFieldErrorProps {
  message: string
  className?: string
  placement?: FloatingFieldBubblePlacement
}

export function FloatingFieldError({
  message,
  className,
  placement = 'input-corner',
}: FloatingFieldErrorProps) {
  return (
    <FloatingFieldBubble
      message={message}
      variant="error"
      className={className}
      placement={placement}
    />
  )
}
