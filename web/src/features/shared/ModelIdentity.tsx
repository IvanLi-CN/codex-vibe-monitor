import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";

const GPT_56_MODEL_IDENTITIES: Record<
  "sol" | "terra" | "luna",
  { iconName: AppIconName; iconColorClassName: string }
> = {
  sol: {
    iconName: "white-balance-sunny",
    iconColorClassName: "text-warning",
  },
  terra: { iconName: "earth", iconColorClassName: "text-success" },
  luna: {
    iconName: "weather-night",
    iconColorClassName: "text-info",
  },
};

const GPT_6_MODEL_IDENTITIES: Record<
  "astra" | "sol" | "luna",
  { iconName: AppIconName; iconColorClassName: string }
> = {
  astra: { iconName: "creation", iconColorClassName: "model-identity-astra" },
  sol: { iconName: "white-balance-sunny", iconColorClassName: "text-warning" },
  luna: { iconName: "weather-night", iconColorClassName: "text-info" },
};

const GPT_56_MODEL_PATTERN = /^gpt-5\.6-(sol|terra|luna)(?:-(\d{4}-\d{2}-\d{2}))?$/i;
const GPT_56_ALIAS_PATTERN = /^gpt-5\.6$/i;
const GPT_6_MODEL_PATTERN = /^gpt-(6(?:\.1)?)-(astra|sol|luna)(?:-(\d{4}-\d{2}-\d{2}))?$/i;

export type ModelIdentityGenerationLabel = "5.6" | "6" | "6.1";

type ResolvedModelIdentity = {
  iconName: AppIconName;
  generation: 5 | 6;
  generationLabel: ModelIdentityGenerationLabel;
  variant: string;
  iconColorClassName?: string;
};

function isCalendarValidDate(value: string) {
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}

function resolveModelIdentity(model: string): ResolvedModelIdentity | null {
  const normalizedModel = model.trim();
  if (GPT_56_ALIAS_PATTERN.test(normalizedModel)) {
    return {
      ...GPT_56_MODEL_IDENTITIES.sol,
      generation: 5,
      generationLabel: "5.6",
      variant: "sol",
    };
  }

  const gpt56Match = GPT_56_MODEL_PATTERN.exec(normalizedModel);
  if (gpt56Match) {
    if (gpt56Match[2] && !isCalendarValidDate(gpt56Match[2])) return null;
    const family = gpt56Match[1].toLowerCase() as keyof typeof GPT_56_MODEL_IDENTITIES;
    return {
      ...GPT_56_MODEL_IDENTITIES[family],
      generation: 5,
      generationLabel: "5.6",
      variant: family,
    };
  }

  const gpt6Match = GPT_6_MODEL_PATTERN.exec(normalizedModel);
  if (!gpt6Match || (gpt6Match[3] && !isCalendarValidDate(gpt6Match[3]))) return null;
  const generationLabel = gpt6Match[1] as "6" | "6.1";
  const family = gpt6Match[2].toLowerCase() as keyof typeof GPT_6_MODEL_IDENTITIES;
  return {
    ...GPT_6_MODEL_IDENTITIES[family],
    generation: 6,
    generationLabel,
    variant: family,
  };
}

export function resolveModelIdentityGeneration(model: string): 5 | 6 | null {
  return resolveModelIdentity(model)?.generation ?? null;
}

export function resolveModelIdentityGenerationLabel(
  model: string,
): ModelIdentityGenerationLabel | null {
  return resolveModelIdentity(model)?.generationLabel ?? null;
}

export function resolveModelIdentityIcon(model: string): AppIconName | null {
  return resolveModelIdentity(model.trim())?.iconName ?? null;
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
  const identity = resolveModelIdentity(resolvedModel);
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
      className={cn(
        "inline-flex flex-none items-center justify-center",
        "h-5 w-5",
        identity.iconColorClassName,
        className,
      )}
      title={resolvedTitle}
      aria-label={resolvedModel}
      role="img"
      data-testid={testId}
      data-model-identity={resolvedModel}
      data-model-icon={identity.iconName}
      data-model-generation={identity.generation}
      data-model-generation-label={identity.generationLabel}
      data-model-variant={identity.variant}
    >
      <AppIcon
        {...iconProps}
        name={identity.iconName}
        className={cn("h-4 w-4", iconClassName, iconProps?.className)}
        aria-hidden
      />
    </span>
  );
}
