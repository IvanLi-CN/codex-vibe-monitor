import type { ReactNode } from "react";
import { cn } from "../../lib/utils";
import type { BubbleVariant } from "./bubble";
import { FloatingFieldBubble } from "./floating-field-bubble";
import { FloatingFieldError } from "./floating-field-error";

interface FormFieldFeedbackProps {
  label: ReactNode;
  labelId?: string;
  message?: string | null;
  messageId?: string;
  variant?: BubbleVariant;
  className?: string;
  labelClassName?: string;
  messageClassName?: string;
}

export function FormFieldFeedback({
  label,
  labelId,
  message,
  messageId,
  variant = "error",
  className,
  labelClassName,
  messageClassName,
}: FormFieldFeedbackProps) {
  const feedback = !message ? null : variant === "error" ? (
    <FloatingFieldError
      id={messageId}
      placement="label-inline"
      message={message}
      className={messageClassName}
    />
  ) : (
    <FloatingFieldBubble
      id={messageId}
      placement="label-inline"
      message={message}
      variant={variant}
      className={messageClassName}
    />
  );

  return (
    <div
      className={cn(
        "flex flex-wrap items-start gap-2 md:min-h-7 md:flex-nowrap md:items-center md:justify-between",
        className,
      )}
    >
      <span id={labelId} className={cn("field-label", labelClassName)}>
        {label}
      </span>
      {feedback}
    </div>
  );
}
