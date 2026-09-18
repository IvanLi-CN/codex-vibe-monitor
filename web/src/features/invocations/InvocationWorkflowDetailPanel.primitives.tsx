import type * as React from "react";
import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import { cn } from "../../lib/utils";
import type { AttemptUsageAudit } from "./InvocationWorkflowDetailPanel.formatters";
import {
  FALLBACK_CELL,
  formatCurrency,
  formatOptionalNumber,
} from "./InvocationWorkflowDetailPanel.formatters";
import {
  renderInvocationCostAuditWarning,
  resolveInvocationCostAuditDisplay,
} from "./invocation-cost-audit";

export function IdentityField({
  label,
  value,
  monospace = false,
}: {
  label: string;
  value: string;
  monospace?: boolean;
}) {
  return (
    <div className="min-w-0">
      <div className="text-[11px] font-medium text-base-content/56">{label}</div>
      <div
        className={cn(
          "mt-1 min-w-0 break-all text-sm text-base-content/84",
          monospace && "font-mono text-[13px]",
        )}
      >
        {value}
      </div>
    </div>
  );
}

export function SummaryRows({
  rows,
  compact = false,
}: {
  rows: Array<{
    label: string;
    value: string;
    variant?: "primary" | "secondary" | "success" | "warning" | "error";
    action?: {
      title: string;
      onClick: () => void;
    };
  }>;
  compact?: boolean;
}) {
  const toneClassFor = (variant?: "primary" | "secondary" | "success" | "warning" | "error") => {
    if (variant === "success") return "tone-ink-success";
    if (variant === "warning") return "tone-ink-warning";
    if (variant === "error") return "tone-ink-error";
    if (variant === "primary") return "tone-ink-info";
    return "text-base-content/88";
  };

  return (
    <dl className="divide-y divide-base-300/42">
      {rows.map((row) => (
        <div
          key={row.label}
          className={cn("flex items-start justify-between gap-4 py-3", compact && "gap-3 py-2.5")}
        >
          <dt
            className={cn("text-[11px] font-medium text-base-content/58", compact && "text-[10px]")}
          >
            {row.label}
          </dt>
          <dd
            className={cn(
              "min-w-0 text-right text-sm font-medium",
              compact && "text-[13px] leading-5",
              toneClassFor(row.variant),
            )}
          >
            {row.action ? (
              <button
                type="button"
                title={row.action.title}
                className={cn(
                  "break-all text-right underline decoration-dotted decoration-current/50 underline-offset-2 transition-colors",
                  row.variant ? toneClassFor(row.variant) : "tone-ink-info",
                  "hover:text-primary focus-visible:outline-none focus-visible:text-primary",
                )}
                onClick={row.action.onClick}
              >
                {row.value}
              </button>
            ) : (
              <span className="break-all">{row.value}</span>
            )}
          </dd>
        </div>
      ))}
    </dl>
  );
}

export function SnapshotMetric({
  label,
  value,
  variant = "secondary",
  compact = false,
}: {
  label: string;
  value: string;
  variant?: "primary" | "secondary" | "success" | "warning" | "error";
  compact?: boolean;
}) {
  return (
    <div
      className={cn(
        "invocation-detail-subsurface rounded-[0.95rem] px-2.5 py-2.5 sm:px-3 sm:py-3",
        compact && "rounded-[0.8rem] px-2 py-2 sm:px-2.5 sm:py-2.5",
      )}
    >
      <div
        className={cn(
          "text-[11px] font-medium text-base-content/56",
          compact && "text-[10px] leading-4",
        )}
      >
        {label}
      </div>
      <div
        className={cn(
          "mt-1 break-all text-sm font-semibold text-base-content",
          compact && "text-[13px] leading-[1.15]",
          variant === "success" && "tone-ink-success",
          variant === "warning" && "tone-ink-warning",
          variant === "error" && "tone-ink-error",
          variant === "primary" && "tone-ink-info",
        )}
      >
        {value}
      </div>
    </div>
  );
}

export function OverviewGrid({
  items,
  className,
}: {
  items: Array<{ label: string; value: string; monospace?: boolean }>;
  className?: string;
}) {
  return (
    <dl className={cn("grid gap-x-5 gap-y-4 md:grid-cols-2 xl:grid-cols-3", className)}>
      {items.map((item) => (
        <div key={`${item.label}-${item.value}`} className="min-w-0">
          <dt className="text-[11px] font-medium text-base-content/58">{item.label}</dt>
          <dd
            className={cn(
              "mt-1 break-all text-sm text-base-content/86",
              item.monospace !== false && "font-mono",
            )}
          >
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

export function DetailInfoPanel({
  title,
  items,
  overviewClassName,
}: {
  title: string;
  items: Array<{ label: string; value: string; monospace?: boolean }>;
  overviewClassName?: string;
}) {
  if (items.length === 0) return null;
  return (
    <section className="invocation-detail-subsurface rounded-[0.95rem] px-3.5 py-3">
      <div className="text-[11px] font-medium text-base-content/56">{title}</div>
      <div className="mt-3">
        <OverviewGrid className={overviewClassName} items={items} />
      </div>
    </section>
  );
}

export function AttemptUsageAuditPanel({
  usageAudit,
  localeTag,
  isZh,
}: {
  usageAudit: AttemptUsageAudit | null;
  localeTag: string;
  isZh: boolean;
}) {
  const { t } = useTranslation();
  if (!usageAudit) return null;

  const totalCostDisplay = resolveInvocationCostAuditDisplay(
    usageAudit.audit,
    usageAudit.recordedCosts?.total ?? null,
  );
  const metricItems = buildAttemptUsageMetricItems(usageAudit, totalCostDisplay, localeTag, isZh);
  const rows = buildAttemptUsageCostRows(usageAudit, totalCostDisplay, isZh);

  return (
    <>
      <DetailMetaStrip items={metricItems} />
      <section className="invocation-detail-subsurface rounded-[0.95rem] px-3.5 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="text-[11px] font-medium text-base-content/56">
            {isZh ? "Token 与成本" : "Token and cost"}
          </div>
          {renderInvocationCostAuditWarning(
            usageAudit.audit,
            t,
            (value) => formatCurrency(value, localeTag),
            { testId: "workflow-usage-cost-warning" },
          )}
        </div>
        <div className="mt-3 space-y-2">
          {rows.map((row) => (
            <div
              key={row.key}
              className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 rounded-xl border border-base-300/60 bg-base-100/72 px-3 py-2 text-xs"
            >
              <div className="min-w-0 text-base-content/68">{row.label}</div>
              <div className="font-mono text-base-content/84">
                {formatCurrency(row.recorded, localeTag)}
              </div>
              <div className="font-mono text-base-content/62">
                {`${isZh ? "本地" : "Local"} ${formatCurrency(row.local, localeTag)}`}
              </div>
            </div>
          ))}
          {!totalCostDisplay.mismatch && totalCostDisplay.reason ? (
            <div className="rounded-xl border border-base-300/55 bg-base-100/65 px-3 py-2 text-xs text-base-content/58">
              {t("records.costAudit.notComparable")}
            </div>
          ) : null}
        </div>
      </section>
    </>
  );
}

export function buildAttemptUsageMetricItems(
  usageAudit: AttemptUsageAudit,
  totalCostDisplay: ReturnType<typeof resolveInvocationCostAuditDisplay>,
  localeTag: string,
  isZh: boolean,
) {
  return [
    {
      label: isZh ? "未命中缓存输入 Token" : "Uncached input tokens",
      value: formatOptionalNumber(usageAudit.cacheWriteTokens, localeTag),
    },
    {
      label: isZh ? "命中缓存输入 Token" : "Cached input tokens",
      value: formatOptionalNumber(usageAudit.cacheInputTokens, localeTag),
    },
    {
      label: isZh ? "输出 Token" : "Output tokens",
      value: formatOptionalNumber(usageAudit.outputTokens, localeTag),
    },
    {
      label: isZh ? "金额" : "Amount",
      value: formatCurrency(totalCostDisplay.recordedTotal, localeTag),
    },
  ].filter((item) => item.value !== FALLBACK_CELL);
}

export function buildAttemptUsageCostRows(
  usageAudit: AttemptUsageAudit,
  totalCostDisplay: ReturnType<typeof resolveInvocationCostAuditDisplay>,
  isZh: boolean,
) {
  return [
    {
      key: "cacheWrite",
      label: isZh ? "未命中缓存输入成本" : "Uncached input cost",
      recorded: usageAudit.recordedCosts?.cacheWrite ?? null,
      local: usageAudit.localCosts?.cacheWrite ?? null,
    },
    {
      key: "cacheRead",
      label: isZh ? "命中缓存输入成本" : "Cached input cost",
      recorded: usageAudit.recordedCosts?.cacheRead ?? null,
      local: usageAudit.localCosts?.cacheRead ?? null,
    },
    {
      key: "output",
      label: isZh ? "输出成本" : "Output cost",
      recorded: usageAudit.recordedCosts?.output ?? null,
      local: usageAudit.localCosts?.output ?? null,
    },
    {
      key: "reasoning",
      label: isZh ? "推理成本" : "Reasoning cost",
      recorded: usageAudit.recordedCosts?.reasoning ?? null,
      local: usageAudit.localCosts?.reasoning ?? null,
    },
    {
      key: "total",
      label: isZh ? "总成本" : "Total cost",
      recorded: totalCostDisplay.recordedTotal,
      local: totalCostDisplay.localTotal,
    },
  ];
}

export function DetailMetaStrip({
  items,
}: {
  items: Array<{
    label: string;
    value: string;
    monospace?: boolean;
    fullWidth?: boolean;
  }>;
}) {
  if (items.length === 0) return null;
  return (
    <section className="invocation-detail-subsurface flex flex-wrap gap-x-4 gap-y-2 rounded-[0.95rem] px-3 py-2.5">
      {items.map((item) => (
        <div
          key={`${item.label}-${item.value}`}
          className={cn(
            "flex min-w-0 items-baseline gap-1.5 text-[12px] leading-5",
            item.fullWidth && "basis-full",
          )}
        >
          <span className="shrink-0 text-[10.5px] font-medium text-base-content/52">
            {item.label}
          </span>
          <span
            className={cn(
              "min-w-0 break-all font-semibold text-base-content/84",
              item.monospace && "font-mono text-[11.5px]",
            )}
          >
            {item.value}
          </span>
        </div>
      ))}
    </section>
  );
}

export function DetailFrame({
  controls,
  children,
}: {
  controls?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="invocation-detail-subsurface min-w-0 max-w-full overflow-hidden space-y-2.5 rounded-b-[1rem] border-t-0 px-3.5 pb-3.5 pt-2.5">
      {controls ? (
        <div className="flex min-w-0 items-center justify-between gap-3">{controls}</div>
      ) : null}
      {children}
    </div>
  );
}

export interface TimelineMetricButtonProps {
  label: string;
  tag?: string | null;
  primary: string;
  secondary?: string | null;
  secondaryTone?: "success";
  tertiary?: string | null;
  tertiaryChips?: string[] | null;
  tertiaryOverflowCount?: number;
  monospace?: boolean;
  active?: boolean;
  onClick: () => void;
}

export function TimelineMetricButtonContent({
  label,
  tag,
  primary,
  secondary,
  secondaryTone,
  tertiary,
  tertiaryChips,
  tertiaryOverflowCount = 0,
  monospace = false,
  active,
}: Omit<TimelineMetricButtonProps, "onClick">) {
  return (
    <div className="flex h-full flex-col gap-1">
      <div className="flex h-5 items-start justify-between gap-2">
        <div
          className={cn(
            "text-[10.5px] font-medium",
            active ? "tone-ink-primary" : "text-base-content/70",
          )}
        >
          {label}
        </div>
        {tag ? (
          <Chip
            size="micro"
            tone={active ? "primary" : "secondary"}
            title={tag}
            className="shrink-0 px-1.5 text-[9.5px] font-semibold"
          >
            {tag}
          </Chip>
        ) : null}
      </div>
      <div className="flex min-h-0 flex-col gap-0.5">
        <div
          title={primary}
          className={cn(
            "overflow-hidden text-[12.5px] font-semibold leading-[1.3] [display:-webkit-box] [-webkit-box-orient:vertical] [-webkit-line-clamp:2] [overflow-wrap:anywhere]",
            monospace && "font-mono text-[12px] leading-[1.35]",
            active ? "tone-ink-primary" : "text-base-content/90",
          )}
        >
          {primary}
        </div>
        {secondary && secondary !== FALLBACK_CELL ? (
          <div
            title={secondary}
            className={cn(
              "overflow-hidden text-[10.5px] leading-4 text-ellipsis whitespace-nowrap",
              secondaryTone === "success" ? "text-success" : "text-base-content/70",
            )}
          >
            {secondary}
          </div>
        ) : null}
        {tertiaryChips && tertiaryChips.length > 0 ? (
          <div className="flex min-w-0 items-center gap-1 overflow-hidden">
            {tertiaryChips.map((chip) => (
              <Chip
                size="micro"
                tone={active ? "primary" : "secondary"}
                key={`${label}-${chip}`}
                title={chip}
                className="shrink-0 px-1.5 text-[9.5px] font-medium"
              >
                {chip}
              </Chip>
            ))}
            {tertiaryOverflowCount > 0 ? (
              <Chip
                size="micro"
                tone={active ? "primary" : "secondary"}
                className="shrink-0 px-1.5 text-[9.5px] font-semibold"
              >
                +{tertiaryOverflowCount}
              </Chip>
            ) : null}
          </div>
        ) : tertiary && tertiary !== FALLBACK_CELL ? (
          <div
            title={tertiary}
            className="overflow-hidden text-[10.5px] leading-4 text-base-content/70 text-ellipsis whitespace-nowrap"
          >
            {tertiary}
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function TimelineMetricButton({
  label,
  tag,
  primary,
  secondary,
  secondaryTone,
  tertiary,
  tertiaryChips,
  tertiaryOverflowCount = 0,
  monospace = false,
  active,
  onClick,
}: TimelineMetricButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        "h-full min-w-0 bg-base-100/84 px-3 py-2.5 text-left transition-[background-color,color] duration-150 focus-visible:z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary",
        active ? "bg-primary/8" : "text-base-content hover:bg-base-100",
      )}
      onClick={onClick}
    >
      <TimelineMetricButtonContent
        label={label}
        tag={tag}
        primary={primary}
        secondary={secondary}
        secondaryTone={secondaryTone}
        tertiary={tertiary}
        tertiaryChips={tertiaryChips}
        tertiaryOverflowCount={tertiaryOverflowCount}
        monospace={monospace}
        active={active}
      />
    </button>
  );
}

export function PayloadNotice({
  tone = "default",
  children,
}: {
  tone?: "default" | "warning" | "error";
  children: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "rounded-xl border px-3 py-3 text-sm",
        tone === "warning" && "border-warning/30 bg-warning/8 text-base-content/74",
        tone === "error" && "border-error/24 bg-error/6 text-base-content/78",
        tone === "default" && "invocation-detail-subsurface text-base-content/64",
      )}
    >
      {children}
    </div>
  );
}
