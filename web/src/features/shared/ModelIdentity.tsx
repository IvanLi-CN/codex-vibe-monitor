import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";

const GPT_56_MODEL_IDENTITIES: Record<
  "sol" | "terra" | "luna",
  { iconName: AppIconName; iconColorClassName: string; generation: 5; variant: string }
> = {
  sol: {
    iconName: "white-balance-sunny",
    iconColorClassName: "text-warning",
    generation: 5,
    variant: "sol",
  },
  terra: { iconName: "earth", iconColorClassName: "text-success", generation: 5, variant: "terra" },
  luna: {
    iconName: "weather-night",
    iconColorClassName: "text-info",
    generation: 5,
    variant: "luna",
  },
};

const GPT_6_MODEL_IDENTITIES: Record<
  "astra" | "sol" | "luna",
  { iconName: AppIconName; generation: 6; variant: string }
> = {
  astra: {
    iconName: "creation",
    generation: 6,
    variant: "astra",
  },
  sol: {
    iconName: "weather-sunny",
    generation: 6,
    variant: "sol",
  },
  luna: {
    iconName: "moon-waning-crescent",
    generation: 6,
    variant: "luna",
  },
};

const GPT_56_MODEL_PATTERN = /^gpt-5\.6-(sol|terra|luna)(?:-\d{4}-\d{2}-\d{2})?$/i;
const GPT_56_ALIAS_PATTERN = /^gpt-5\.6$/i;
const GPT_6_MODEL_PATTERN = /^gpt-6-(astra|sol|luna)(?:-(\d{4}-\d{2}-\d{2}))?$/i;

type ModelIdentityPresentation =
  | (typeof GPT_56_MODEL_IDENTITIES)[keyof typeof GPT_56_MODEL_IDENTITIES]
  | (typeof GPT_6_MODEL_IDENTITIES)[keyof typeof GPT_6_MODEL_IDENTITIES];

export type ModelIdentityPresentationMode = "standalone" | "embedded" | "compact";

function isCalendarValidDate(value: string) {
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}

function resolveModelIdentityPresentation(model: string): ModelIdentityPresentation | null {
  if (GPT_56_ALIAS_PATTERN.test(model)) return GPT_56_MODEL_IDENTITIES.sol;
  const gpt56Match = GPT_56_MODEL_PATTERN.exec(model);
  if (gpt56Match) {
    return GPT_56_MODEL_IDENTITIES[
      gpt56Match[1].toLowerCase() as keyof typeof GPT_56_MODEL_IDENTITIES
    ];
  }

  const gpt6Match = GPT_6_MODEL_PATTERN.exec(model);
  if (!gpt6Match || (gpt6Match[2] && !isCalendarValidDate(gpt6Match[2]))) return null;
  return GPT_6_MODEL_IDENTITIES[gpt6Match[1].toLowerCase() as keyof typeof GPT_6_MODEL_IDENTITIES];
}

export function resolveModelIdentityGeneration(model: string): 5 | 6 | null {
  return resolveModelIdentityPresentation(model)?.generation ?? null;
}

export function resolveModelIdentityIcon(model: string): AppIconName | null {
  return resolveModelIdentityPresentation(model.trim())?.iconName ?? null;
}

export interface ModelIdentityProps {
  model: string;
  className?: string;
  textClassName?: string;
  iconClassName?: string;
  presentation?: ModelIdentityPresentationMode;
  title?: string;
  testId?: string;
  iconProps?: Omit<ComponentProps<typeof AppIcon>, "name">;
}

export function ModelIdentity({
  model,
  className,
  textClassName,
  iconClassName,
  presentation = "standalone",
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

  const isGpt6 = identity.generation === 6;
  const resolvedPresentation = isGpt6 ? presentation : undefined;

  return (
    <span
      className={cn(
        "inline-flex flex-none items-center justify-center",
        "h-5 w-5",
        isGpt6 ? "model-identity-gpt6" : undefined,
        className,
      )}
      title={resolvedTitle}
      aria-label={resolvedModel}
      role="img"
      data-testid={testId}
      data-model-identity={resolvedModel}
      data-model-icon={identity.iconName}
      data-model-generation={identity.generation}
      data-model-variant={identity.variant}
      data-model-presentation={resolvedPresentation}
    >
      <AppIcon
        {...iconProps}
        name={identity.iconName}
        className={cn(
          "h-4 w-4",
          iconClassName,
          iconProps?.className,
          identity.generation === 5 ? identity.iconColorClassName : undefined,
        )}
        aria-hidden
      />
    </span>
  );
}
