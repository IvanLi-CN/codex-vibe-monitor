import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";

const GPT_56_MODEL_ICON_NAMES: Record<string, AppIconName> = {
  sol: "white-balance-sunny",
  terra: "earth",
  luna: "weather-night",
};

const GPT_56_MODEL_PATTERN = /^gpt-5\.6-(sol|terra|luna)(?:-\d{4}-\d{2}-\d{2})?$/i;

export function resolveModelIdentityIcon(model: string): AppIconName | null {
  const match = GPT_56_MODEL_PATTERN.exec(model.trim());
  if (!match) return null;
  return GPT_56_MODEL_ICON_NAMES[match[1].toLowerCase()] ?? null;
}

export interface ModelIdentityProps {
  model: string;
  className?: string;
  textClassName?: string;
  iconClassName?: string;
  title?: string;
  testId?: string;
  iconProps?: Omit<ComponentProps<typeof AppIcon>, "name">;
}

export function ModelIdentity({
  model,
  className,
  textClassName,
  iconClassName,
  title,
  testId,
  iconProps,
}: ModelIdentityProps) {
  const resolvedModel = model.trim();
  const iconName = resolveModelIdentityIcon(resolvedModel);
  const resolvedTitle = title ?? resolvedModel;

  if (!iconName) {
    return (
      <span
        className={cn("min-w-0 max-w-full truncate leading-none", className, textClassName)}
        title={resolvedTitle || undefined}
        data-testid={testId}
        data-model-identity={resolvedModel || undefined}
      >
        {model}
      </span>
    );
  }

  return (
    <span
      className={cn("inline-flex h-5 w-5 flex-none items-center justify-center", className)}
      title={resolvedTitle}
      aria-label={resolvedModel}
      role="img"
      data-testid={testId}
      data-model-identity={resolvedModel}
      data-model-icon={iconName}
    >
      <AppIcon
        {...iconProps}
        name={iconName}
        className={cn("h-4 w-4", iconClassName, iconProps?.className)}
        aria-hidden
      />
    </span>
  );
}
