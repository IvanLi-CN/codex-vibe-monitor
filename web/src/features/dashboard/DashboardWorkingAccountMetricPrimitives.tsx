import type { ReactNode } from "react";
import { useLayoutEffect, useRef, useState } from "react";
import { Tooltip } from "../../components/ui/tooltip";
import type { ModelPerformance } from "../../lib/api";
import { cn } from "../../lib/utils";
import { FALLBACK_CELL } from "../invocations/invocation-details-shared";
import { AdaptiveDisplayValue } from "../shared/AdaptiveMetricValue";
import { AppIcon, type AppIconName } from "../shared/AppIcon";
import {
  ACCOUNT_INLINE_METRIC_ICON_AND_GAP_PX,
  ACCOUNT_METRIC_DOT_TONE_CLASSNAMES,
  ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES,
  type AccountDisplayValue,
  type AccountMetricDetailSection,
  type AccountMetricTone,
} from "./DashboardWorkingAccountMetrics";
import { ACCOUNT_CARD_INNER_RING_CLASS_NAME } from "./DashboardWorkingConversationsSection";
import { ModelPerformanceTrigger } from "./ModelPerformanceTrigger";

type AccountMetricToneWithNeutral = Exclude<AccountMetricTone, "neutral"> | "neutral";

function AccountMetricDetailTooltip({
  label,
  value,
  valueClassName,
  sections,
}: {
  label: string;
  value: string;
  valueClassName: string;
  sections: AccountMetricDetailSection[];
}) {
  return (
    <div data-testid="dashboard-upstream-account-metric-tooltip" className="space-y-3">
      <div className="flex min-w-0 items-baseline justify-between gap-4 border-b border-base-300/45 pb-2">
        <div className="min-w-0 text-[11px] font-semibold leading-4 text-base-content/62">
          {label}
        </div>
        <div
          className={cn(
            "min-w-0 truncate text-right font-mono text-[1rem] font-semibold leading-none",
            valueClassName,
          )}
        >
          {value}
        </div>
      </div>
      {sections.map((section) => (
        <div key={section.title} className="space-y-1.5">
          <div className="text-[10px] font-semibold leading-4 text-base-content/52">
            {section.title}
          </div>
          <div className="space-y-1">
            {section.rows.map((row) => (
              <div
                key={`${section.title}:${row.label}`}
                className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-baseline gap-3"
              >
                <span className="min-w-0 truncate text-[11px] leading-4 text-base-content/68">
                  {row.label}
                </span>
                <span
                  className={cn(
                    "min-w-0 max-w-[12rem] truncate text-right font-mono text-[11px] font-semibold leading-4 text-base-content",
                    row.tone ? ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[row.tone] : null,
                  )}
                >
                  {row.value}
                </span>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function AccountHeroMetricCard({
  label,
  value,
  valueClassName,
  iconClassName,
  iconName,
  hint,
  metricKey,
  children,
}: {
  label: string;
  value: AccountDisplayValue;
  valueClassName: string;
  iconClassName: string;
  iconName: AppIconName;
  hint?: string;
  metricKey?: string;
  children?: ReactNode;
}) {
  return (
    <div
      data-testid="dashboard-upstream-account-metric-card"
      data-metric={metricKey}
      data-motion-surface
      className={cn(
        "h-full w-full rounded-[0.85rem] px-3 py-2.5 transition-colors duration-200",
        "bg-base-100/72 ring-1 ring-inset",
        ACCOUNT_CARD_INNER_RING_CLASS_NAME,
      )}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/54">
        {label}
      </div>
      <div className="mt-1 flex min-w-0 items-center gap-1.5">
        <span
          aria-hidden
          data-testid={metricKey ? `dashboard-upstream-account-${metricKey}-icon` : undefined}
          className={cn(
            "flex h-[1.35rem] w-[1.35rem] shrink-0 items-center justify-center text-[1.22rem] leading-none",
            iconClassName,
          )}
        >
          <AppIcon name={iconName} className={cn(iconName === "send" && "-rotate-45")} />
        </span>
        <div
          className={cn(
            "min-w-0 flex-1 overflow-hidden text-ellipsis font-mono text-[1.08rem] font-semibold leading-none",
            valueClassName,
          )}
        >
          <AdaptiveDisplayValue
            spec={value.spec}
            className="block min-w-0 max-w-full"
            data-testid={metricKey ? `dashboard-upstream-account-${metricKey}-value` : undefined}
            animateDigits
          />
        </div>
      </div>
      {hint ? <div className="mt-1 text-[11px] leading-4 text-base-content/58">{hint}</div> : null}
      {children ? <div className="mt-1.5">{children}</div> : null}
    </div>
  );
}

export function AccountHeroMetric({
  label,
  value,
  tone,
  iconName,
  hint,
  detailSections,
  tooltipContent,
  metricKey,
  children,
}: {
  label: string;
  value: AccountDisplayValue;
  tone: AccountMetricToneWithNeutral;
  iconName: AppIconName;
  hint?: string;
  detailSections?: AccountMetricDetailSection[];
  tooltipContent?: ReactNode;
  metricKey?: string;
  children?: ReactNode;
}) {
  const valueClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/55"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone];
  const iconClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/45"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone];
  const card = (
    <AccountHeroMetricCard
      label={label}
      value={value}
      valueClassName={valueClassName}
      iconClassName={iconClassName}
      iconName={iconName}
      hint={hint}
      metricKey={metricKey}
    >
      {children}
    </AccountHeroMetricCard>
  );
  if (!detailSections?.length && !tooltipContent) return card;
  return (
    <Tooltip
      clickToOpen
      side="top"
      sideOffset={12}
      triggerElement="div"
      className="h-full w-full rounded-[0.85rem] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
      contentClassName={
        tooltipContent
          ? "max-w-[min(42rem,calc(100vw-1rem))] w-[min(42rem,calc(100vw-1rem))] px-3.5 py-3"
          : "w-[min(21rem,calc(100vw-1rem))] px-3.5 py-3"
      }
      content={
        tooltipContent ?? (
          <AccountMetricDetailTooltip
            label={label}
            value={value.fullText}
            valueClassName={valueClassName}
            sections={detailSections ?? []}
          />
        )
      }
      triggerProps={{ tabIndex: 0, "aria-label": `${label} ${value.ariaText}` }}
    >
      {card}
    </Tooltip>
  );
}

function useAvailableMetricWidth(
  fillAvailableWidth: boolean,
  widthMeasureRef: React.RefObject<HTMLSpanElement | null>,
) {
  const [availableWidthPx, setAvailableWidthPx] = useState<number | undefined>(undefined);
  useLayoutEffect(() => {
    if (!fillAvailableWidth) {
      setAvailableWidthPx(undefined);
      return undefined;
    }
    const element = widthMeasureRef.current;
    if (!element) return undefined;
    const syncWidth = () => {
      const nextWidth = Math.max(0, element.clientWidth - ACCOUNT_INLINE_METRIC_ICON_AND_GAP_PX);
      setAvailableWidthPx((current) => (current === nextWidth ? current : nextWidth));
    };
    syncWidth();
    window.addEventListener("resize", syncWidth);
    if (typeof ResizeObserver === "undefined") {
      return () => window.removeEventListener("resize", syncWidth);
    }
    const observer = new ResizeObserver(syncWidth);
    observer.observe(element);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", syncWidth);
    };
  }, [fillAvailableWidth, widthMeasureRef]);
  return availableWidthPx;
}

function AccountInlineMetricVisual({
  value,
  valueClassName,
  iconClassName,
  iconName,
  iconAdjustmentClassName,
  availableWidthPx,
  valueWidthBudgetCh,
  fillAvailableWidth,
  valueTestId,
  className,
}: {
  value: AccountDisplayValue;
  valueClassName: string;
  iconClassName: string;
  iconName: AppIconName;
  iconAdjustmentClassName: string | null;
  availableWidthPx?: number;
  valueWidthBudgetCh?: number;
  fillAvailableWidth: boolean;
  valueTestId?: string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-[1.35rem] min-w-0 max-w-full shrink-0 items-center gap-1.5 whitespace-nowrap",
        className,
      )}
    >
      <span
        aria-hidden
        className={cn(
          "flex h-[1.2rem] w-[1.2rem] shrink-0 items-center justify-center text-[1.05rem] leading-none",
          iconClassName,
        )}
      >
        <AppIcon name={iconName} className={iconAdjustmentClassName ?? undefined} />
      </span>
      <span
        className={cn(
          "inline-flex h-[1.2rem] min-w-0 items-center overflow-hidden font-mono text-[1.02rem] font-semibold leading-none",
          valueClassName,
        )}
      >
        <AdaptiveDisplayValue
          spec={value.spec}
          className="block min-w-0 max-w-full"
          availableWidthPx={availableWidthPx}
          maxWidthCh={fillAvailableWidth ? undefined : valueWidthBudgetCh}
          data-testid={valueTestId}
          animateDigits
        />
      </span>
    </span>
  );
}

function AccountInlineMetricTrigger({
  children,
  modelPerformance,
  modelPerformanceTitle,
  modelPerformanceAriaLabel,
  triggerAriaLabel,
  fillAvailableWidth,
  triggerAlignmentClassName,
}: {
  children: ReactNode;
  modelPerformance?: ModelPerformance | null;
  modelPerformanceTitle?: string;
  modelPerformanceAriaLabel: string;
  triggerAriaLabel: string;
  fillAvailableWidth: boolean;
  triggerAlignmentClassName: string;
}) {
  const className = cn(
    "rounded-md",
    fillAvailableWidth ? `w-full ${triggerAlignmentClassName}` : null,
  );
  if (modelPerformance?.available && modelPerformanceTitle) {
    return (
      <ModelPerformanceTrigger
        title={modelPerformanceTitle}
        ariaLabel={modelPerformanceAriaLabel}
        performance={modelPerformance}
        className={className}
      >
        {children}
      </ModelPerformanceTrigger>
    );
  }
  return (
    <Tooltip
      content={<span className="font-medium">{modelPerformanceTitle}</span>}
      clickToOpen
      className={className}
      triggerProps={{ tabIndex: 0, "aria-label": triggerAriaLabel }}
    >
      {children}
    </Tooltip>
  );
}

export function AccountInlineMetric({
  label,
  value,
  tone,
  iconName,
  metricKey,
  className,
  alignment = "start",
  fillAvailableWidth = false,
  valueWidthBudgetCh,
  modelPerformance,
  modelPerformanceTitle,
}: {
  label: string;
  value: AccountDisplayValue;
  tone: AccountMetricTone;
  iconName: AppIconName;
  metricKey?: string;
  className?: string;
  alignment?: "start" | "center" | "end";
  fillAvailableWidth?: boolean;
  valueWidthBudgetCh?: number;
  modelPerformance?: ModelPerformance | null;
  modelPerformanceTitle?: string;
}) {
  const widthMeasureRef = useRef<HTMLSpanElement | null>(null);
  const availableWidthPx = useAvailableMetricWidth(fillAvailableWidth, widthMeasureRef);
  const valueClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/55"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone];
  const iconClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/45"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone];
  const iconAdjustmentClassName =
    iconName === "send"
      ? "-rotate-45 -translate-y-[0.5px]"
      : iconName === "speedometer"
        ? "-translate-y-px"
        : iconName === "cash-clock"
          ? "translate-y-[0.5px]"
          : null;
  const triggerAlignmentClassName =
    alignment === "center"
      ? "justify-center"
      : alignment === "end"
        ? "justify-end"
        : "justify-start";
  const triggerAriaLabel = `${label} ${value.ariaText}`;
  const valueTestId = metricKey
    ? `dashboard-upstream-account-inline-${metricKey}-value`
    : undefined;
  const slotTestId = metricKey ? `dashboard-upstream-account-inline-${metricKey}-slot` : undefined;
  return (
    <span
      ref={fillAvailableWidth ? widthMeasureRef : undefined}
      data-testid={slotTestId}
      className={cn("min-w-0", fillAvailableWidth ? "block w-full" : "inline-block max-w-full")}
    >
      <AccountInlineMetricTrigger
        modelPerformance={modelPerformance}
        modelPerformanceTitle={modelPerformanceTitle}
        modelPerformanceAriaLabel={`${triggerAriaLabel} ${modelPerformanceTitle}`}
        triggerAriaLabel={triggerAriaLabel}
        fillAvailableWidth={fillAvailableWidth}
        triggerAlignmentClassName={triggerAlignmentClassName}
      >
        <AccountInlineMetricVisual
          value={value}
          valueClassName={valueClassName}
          iconClassName={iconClassName}
          iconName={iconName}
          iconAdjustmentClassName={iconAdjustmentClassName}
          availableWidthPx={availableWidthPx}
          valueWidthBudgetCh={valueWidthBudgetCh}
          fillAvailableWidth={fillAvailableWidth}
          valueTestId={valueTestId}
          className={className}
        />
      </AccountInlineMetricTrigger>
    </span>
  );
}

function AccountSegmentItem({
  segment,
  showLabel,
  showIconWhenLabelHidden,
}: {
  segment: {
    label: string;
    value: AccountDisplayValue;
    tone: AccountMetricTone;
    iconName?: AppIconName;
  };
  showLabel: boolean;
  showIconWhenLabelHidden: boolean;
}) {
  const icon = segment.iconName ? (
    <AppIcon
      name={segment.iconName}
      className={cn("h-3.5 w-3.5 shrink-0", ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[segment.tone])}
      aria-hidden
    />
  ) : (
    <span
      className={cn("h-1.5 w-1.5 rounded-full", ACCOUNT_METRIC_DOT_TONE_CLASSNAMES[segment.tone])}
      aria-hidden="true"
    />
  );
  return (
    <span
      data-testid="dashboard-upstream-account-segment"
      data-motion-surface
      className="inline-flex items-center gap-1.5 whitespace-nowrap rounded-md px-1 py-0.5"
    >
      {showLabel ? icon : showIconWhenLabelHidden && segment.iconName ? icon : null}
      {showLabel ? (
        <span className="text-[11px] font-semibold leading-none text-base-content/72">
          {segment.label}
        </span>
      ) : null}
      <span
        className={cn(
          "min-w-0 font-mono text-[12px] font-semibold leading-none",
          ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[segment.tone],
        )}
      >
        <AdaptiveDisplayValue spec={segment.value.spec} className="block min-w-0 max-w-full" />
      </span>
    </span>
  );
}

export function AccountSegmentList({
  segments,
  className,
  testId,
  showLabel = false,
  showIconWhenLabelHidden = false,
  enableTooltips = true,
}: {
  segments: Array<{
    label: string;
    value: AccountDisplayValue;
    tone: AccountMetricTone;
    iconName?: AppIconName;
  }>;
  className?: string;
  testId?: string;
  showLabel?: boolean;
  showIconWhenLabelHidden?: boolean;
  enableTooltips?: boolean;
}) {
  const renderedSegments = segments.map((segment) => (
    <AccountSegmentItem
      key={`${segment.label}-${segment.value.fullText}`}
      segment={segment}
      showLabel={showLabel}
      showIconWhenLabelHidden={showIconWhenLabelHidden}
    />
  ));
  return (
    <fieldset
      data-testid={testId}
      aria-label={segments
        .map((segment) => `${segment.label} ${segment.value.ariaText}`)
        .join(" · ")}
      className={cn(
        "flex flex-wrap items-center gap-x-3 gap-y-1.5",
        showLabel && "gap-x-4",
        className,
      )}
    >
      {enableTooltips
        ? segments.map((segment, index) => (
            <Tooltip
              key={`${segment.label}-${segment.value.fullText}`}
              content={
                <span className="font-medium">
                  {segment.label}
                  <span className="font-mono font-semibold"> {segment.value.fullText}</span>
                </span>
              }
              clickToOpen
              className="rounded-md"
              triggerProps={{
                tabIndex: 0,
                "aria-label": `${segment.label} ${segment.value.ariaText}`,
              }}
            >
              {renderedSegments[index]}
            </Tooltip>
          ))
        : renderedSegments}
    </fieldset>
  );
}
