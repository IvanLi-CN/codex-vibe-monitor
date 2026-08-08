import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";

const GPT_56_MODEL_IDENTITIES: Record<
  "sol" | "terra" | "luna",
  { iconName: AppIconName; iconColorClassName: string }
> = {
  sol: { iconName: "white-balance-sunny", iconColorClassName: "text-warning" },
  terra: { iconName: "earth", iconColorClassName: "text-success" },
  luna: { iconName: "weather-night", iconColorClassName: "text-info" },
};

const GPT_56_MODEL_PATTERN = /^gpt-5\.6-(sol|terra|luna)(?:-\d{4}-\d{2}-\d{2})?$/i;
const GPT_56_ALIAS_PATTERN = /^gpt-5\.6$/i;

type ModelIdentityPresentation =
  (typeof GPT_56_MODEL_IDENTITIES)[keyof typeof GPT_56_MODEL_IDENTITIES];

function resolveModelIdentityPresentation(model: string): ModelIdentityPresentation | null {
  if (GPT_56_ALIAS_PATTERN.test(model)) return GPT_56_MODEL_IDENTITIES.sol;
  const match = GPT_56_MODEL_PATTERN.exec(model);
  if (!match) return null;
  return GPT_56_MODEL_IDENTITIES[match[1].toLowerCase() as keyof typeof GPT_56_MODEL_IDENTITIES];
}

export function resolveModelIdentityIcon(model: string): AppIconName | null {
  return resolveModelIdentityPresentation(model.trim())?.iconName ?? null;
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
  const identity = resolveModelIdentityPresentation(resolvedModel);
  const resolvedTitle = title ?? resolvedModel;

  if (!identity) {
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
      data-model-icon={identity.iconName}
    >
      <AppIcon
        {...iconProps}
        name={identity.iconName}
        className={cn("h-4 w-4", iconClassName, iconProps?.className, identity.iconColorClassName)}
        aria-hidden
      />
    </span>
  );
}
