import { Button } from "../components/ui/button";
import { Chip } from "../components/ui/chip";
import { Input } from "../components/ui/input";
import { cn } from "../lib/utils";

export type PricingSettingsEntry = {
  model: string;
  inputPer1m: string;
  outputPer1m: string;
  cacheReadPer1m: string;
  cacheWritePer1m: string;
  reasoningPer1m: string;
  source: string;
};

type PricingField = keyof Pick<
  PricingSettingsEntry,
  "model" | "inputPer1m" | "outputPer1m" | "cacheReadPer1m" | "cacheWritePer1m" | "reasoningPer1m"
>;
type Translate = (key: string) => string;
type ChangeField = (index: number, field: PricingField, value: string) => void;

export function pricingEntryKey(entry: PricingSettingsEntry): string {
  return `pricing-${entry.model}-${entry.source}-${entry.inputPer1m}-${entry.outputPer1m}-${entry.cacheReadPer1m}-${entry.cacheWritePer1m}-${entry.reasoningPer1m}`;
}

const mobileFields: Array<{ field: PricingField; label: string; type: "text" | "number" }> = [
  { field: "model", label: "model", type: "text" },
  { field: "inputPer1m", label: "input", type: "number" },
  { field: "outputPer1m", label: "output", type: "number" },
  { field: "cacheReadPer1m", label: "cacheRead", type: "number" },
  { field: "reasoningPer1m", label: "reasoning", type: "number" },
];

function sourceTone(source: string): "success" | "warning" | "secondary" {
  if (source === "official") return "success";
  if (source === "temporary") return "warning";
  return "secondary";
}

export function PricingMobileEntries({
  entries,
  onChange,
  onRemove,
  onBlur,
  t,
}: {
  entries: PricingSettingsEntry[];
  onChange: ChangeField;
  onRemove: (index: number) => void;
  onBlur: () => void;
  t: Translate;
}) {
  return (
    <>
      {entries.map((entry, index) => (
        <PricingMobileEntry
          key={pricingEntryKey(entry)}
          entry={entry}
          index={index}
          onChange={onChange}
          onRemove={onRemove}
          onBlur={onBlur}
          t={t}
        />
      ))}
    </>
  );
}

function PricingMobileEntry({
  entry,
  index,
  onChange,
  onRemove,
  onBlur,
  t,
}: {
  entry: PricingSettingsEntry;
  index: number;
  onChange: ChangeField;
  onRemove: (index: number) => void;
  onBlur: () => void;
  t: Translate;
}) {
  return (
    <article className="surface-subtle rounded-xl p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 space-y-2">
          <div className="text-sm font-semibold text-base-content">
            {entry.model || t("settings.pricing.columns.model")}
          </div>
          <Chip tone={sourceTone(entry.source)} className="inline-flex min-w-[5rem] justify-center">
            {entry.source}
          </Chip>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-8 px-2.5 text-error hover:bg-error/10"
          onClick={() => onRemove(index)}
        >
          {t("settings.pricing.remove")}
        </Button>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        {mobileFields.map(({ field, label, type }) => (
          <label
            key={field}
            className={cn("space-y-2", field === "reasoningPer1m" && "sm:col-span-2")}
            htmlFor={`pricing-mobile-${index}-${label}`}
          >
            <span className="block text-xs font-medium text-base-content/68">
              {t(`settings.pricing.columns.${label}`)}
            </span>
            <Input
              id={`pricing-mobile-${index}-${label}`}
              type={type}
              step={type === "number" ? "any" : undefined}
              className="h-9 px-3"
              value={entry[field]}
              onChange={(event) => onChange(index, field, event.target.value)}
              onBlur={onBlur}
            />
          </label>
        ))}
      </div>
    </article>
  );
}

export function PricingDesktopEntry({
  entry,
  index,
  bodyClassName,
  onChange,
  onRemove,
  onBlur,
  t,
}: {
  entry: PricingSettingsEntry;
  index: number;
  bodyClassName: string;
  onChange: ChangeField;
  onRemove: (index: number) => void;
  onBlur: () => void;
  t: Translate;
}) {
  const fields: PricingField[] = [
    "model",
    "inputPer1m",
    "outputPer1m",
    "cacheReadPer1m",
    "cacheWritePer1m",
    "reasoningPer1m",
  ];
  return (
    <tr
      key={pricingEntryKey(entry)}
      className={cn(
        "transition-colors",
        index % 2 === 0 ? "bg-base-100/38" : "bg-base-200/22",
        "hover:bg-primary/6",
      )}
    >
      {fields.map((field) => (
        <td key={field} className={bodyClassName}>
          <Input
            type={field === "model" ? "text" : "number"}
            step={field === "model" ? undefined : "any"}
            className="h-9 px-3"
            value={entry[field]}
            onChange={(event) => onChange(index, field, event.target.value)}
            onBlur={onBlur}
          />
        </td>
      ))}
      <td className={cn(bodyClassName, "whitespace-nowrap")}>
        <Chip tone={sourceTone(entry.source)} className="inline-flex min-w-[5rem] justify-center">
          {entry.source}
        </Chip>
      </td>
      <td className={cn(bodyClassName, "text-right whitespace-nowrap")}>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-8 px-2.5 text-error hover:bg-error/10"
          onClick={() => onRemove(index)}
        >
          {t("settings.pricing.remove")}
        </Button>
      </td>
    </tr>
  );
}
