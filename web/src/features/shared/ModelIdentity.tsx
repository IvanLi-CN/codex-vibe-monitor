import type { ComponentProps } from "react";
import { cn } from "../../lib/utils";
import { AppIcon, type AppIconName } from "./AppIcon";

const GPT_56_MODEL_ICON_NAMES: Record<string, AppIconName> = {
  sol: "white-balance-sunny",
  terra: "earth",
  luna: "weather-night",
};

const GPT_56_MODEL_PATTERN = /^gpt-5\.6-(sol|terra|luna)(?:-\d{4}-\d{2}-\d{2})?$/i;
const GPT_56_MODEL_FAMILY_ORDER = ["sol", "terra", "luna"] as const;

export type Gpt56ModelFamily = (typeof GPT_56_MODEL_FAMILY_ORDER)[number];

export function resolveGpt56ModelFamily(model: string): Gpt56ModelFamily | null {
  const match = GPT_56_MODEL_PATTERN.exec(model.trim());
  return (match?.[1]?.toLowerCase() as Gpt56ModelFamily | undefined) ?? null;
}

export function resolveModelIdentityIcon(model: string): AppIconName | null {
  const family = resolveGpt56ModelFamily(model);
  return family ? GPT_56_MODEL_ICON_NAMES[family] : null;
}

export function isCompleteGpt56ModelSet(models: readonly string[]): boolean {
  const families = new Set(
    models
      .map(resolveGpt56ModelFamily)
      .filter((family): family is Gpt56ModelFamily => family != null),
  );
  return GPT_56_MODEL_FAMILY_ORDER.every((family) => families.has(family));
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

export interface ModelIdentityGroupProps {
  models: readonly string[];
  className?: string;
  iconClassName?: string;
  title?: string;
  testId?: string;
}

/** A single visual unit for the complete Sol/Terra/Luna model family. */
export function ModelIdentityGroup({
  models,
  className,
  iconClassName,
  title,
  testId,
}: ModelIdentityGroupProps) {
  const targetModels = models.filter((model) => resolveGpt56ModelFamily(model) != null);
  if (!isCompleteGpt56ModelSet(targetModels)) return null;

  const orderedModels = GPT_56_MODEL_FAMILY_ORDER.flatMap((family) =>
    targetModels.filter((model) => resolveGpt56ModelFamily(model) === family),
  );
  const modelKeyCounts = new Map<string, number>();
  const accessibleModelIds = orderedModels.join(" · ");
  const resolvedTitle = title ?? accessibleModelIds;

  return (
    <span
      className={cn(
        "inline-flex h-7 shrink-0 items-center gap-0.5 rounded-full border border-base-300/80 bg-base-100/80 px-1.5 shadow-sm",
        className,
      )}
      title={resolvedTitle}
      aria-label={accessibleModelIds}
      role="group"
      data-testid={testId}
      data-model-identity-group="gpt-5.6"
    >
      {orderedModels.map((model) => {
        const occurrence = modelKeyCounts.get(model) ?? 0;
        modelKeyCounts.set(model, occurrence + 1);
        return (
          <ModelIdentity
            key={`${model}-${occurrence}`}
            model={model}
            className="h-5 w-5 rounded-full bg-base-200/75"
            iconClassName={iconClassName ?? "h-3.5 w-3.5"}
            testId={testId ? `${testId}-${model}-${occurrence}` : undefined}
          />
        );
      })}
    </span>
  );
}
