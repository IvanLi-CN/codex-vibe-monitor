import { FloatingFieldBubble, type FloatingFieldBubblePlacement } from "./floating-field-bubble";

interface FloatingFieldErrorProps {
  id?: string;
  message: string;
  className?: string;
  placement?: FloatingFieldBubblePlacement;
}

export function FloatingFieldError({
  id,
  message,
  className,
  placement = "input-corner",
}: FloatingFieldErrorProps) {
  return (
    <FloatingFieldBubble
      id={id}
      message={message}
      variant="error"
      className={className}
      placement={placement}
    />
  );
}
