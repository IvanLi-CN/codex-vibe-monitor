import { useVirtualizer, useWindowVirtualizer } from "@tanstack/react-virtual";
import {
  Fragment,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Badge } from "../../components/ui/badge";
import type { TranslationKey } from "../../i18n";
import { useTranslation } from "../../i18n";
import type { ApiInvocation } from "../../lib/api";
import {
  type FastIndicatorState,
  type InvocationEndpointDisplay,
  type InvocationImageIntentDisplay,
  invocationStableDomKey,
  invocationStableKey,
} from "../../lib/invocation";
import { resolveInvocationLivePhase } from "../../lib/invocationPhase";
import { resolveInvocationDisplayStatus } from "../../lib/invocationStatus";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import { ListBodyState } from "../shared/ListBodyState";
import { InvocationPhaseBadge } from "./InvocationPhaseBadge";
import {
  buildInvocationDetailViewModel,
  FALLBACK_CELL,
  INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
  InvocationExpandedDetails,
  renderEndpointSummary,
  renderFastIndicator,
  renderImageIntentBadge,
  renderInvocationModelBadge,
  renderInvocationModelRoutingSummary,
  renderReasoningEffortBadge,
  useInvocationPoolAttempts,
} from "./invocation-details-shared";
import { renderInvocationTransportBadge } from "./invocation-transport-badge";

interface InvocationTableProps {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel?: string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  scrollElement?: HTMLElement | null;
  showInvokeId?: boolean;
  scrollTarget?: { invokeId: string; attemptId?: string | null; version: number } | null;
}

type StatusMeta = {
  variant: "default" | "secondary" | "success" | "warning" | "error";
  labelKey?: TranslationKey;
  label?: string;
};

const STATUS_META: Record<string, { variant: StatusMeta["variant"]; labelKey: TranslationKey }> = {
  success: { variant: "success", labelKey: "table.status.success" },
  completed: { variant: "success", labelKey: "table.status.success" },
  failed: { variant: "error", labelKey: "table.status.failed" },
  interrupted: { variant: "error", labelKey: "table.status.interrupted" },
  running: { variant: "default", labelKey: "table.status.running" },
  pending: { variant: "warning", labelKey: "table.status.pending" },
};

const INVOCATION_ID_BASE_FONT_SIZE_PX = 10;

function FittedInvocationId({ invokeId, className }: { invokeId: string; className?: string }) {
  const containerRef = useRef<HTMLSpanElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);

  const fitText = useCallback(() => {
    const container = containerRef.current;
    const text = textRef.current;
    if (!container || !text) return;

    text.style.fontSize = `${INVOCATION_ID_BASE_FONT_SIZE_PX}px`;
    const availableWidth = container.clientWidth;
    const requiredWidth = text.scrollWidth;
    if (availableWidth <= 0 || requiredWidth <= availableWidth) return;

    const fittedSize = INVOCATION_ID_BASE_FONT_SIZE_PX * (availableWidth / requiredWidth) * 0.98;
    text.style.fontSize = `${Math.max(1, fittedSize)}px`;
  }, []);

  useLayoutEffect(() => {
    fitText();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(fitText);
    if (containerRef.current) observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, [fitText]);

  return (
    <span
      ref={containerRef}
      className={cn(
        "block min-w-0 max-w-full overflow-hidden whitespace-nowrap leading-tight",
        className,
      )}
      data-testid="invocation-id"
      title={invokeId}
    >
      <span ref={textRef} className="inline-block whitespace-nowrap">
        {invokeId}
      </span>
    </span>
  );
}

function formatStatusLabel(status: string) {
  const normalized = status.trim();
  if (!normalized) return null;
  const lower = normalized.toLowerCase();
  if (lower.startsWith("http_")) {
    const code = lower.slice("http_".length);
    if (/^\d{3}$/.test(code)) return `HTTP ${code}`;
    return normalized.toUpperCase().replace("_", " ");
  }
  return normalized;
}

function resolveStatusMeta(status?: string | null): StatusMeta {
  const raw = (status ?? "").trim();
  const lower = raw.toLowerCase();
  const known = STATUS_META[lower];
  if (known) return known;
  if (!raw) return { variant: "secondary", labelKey: "table.status.unknown" };
  if (lower.startsWith("http_4"))
    return { variant: "warning", label: formatStatusLabel(raw) ?? raw };
  if (lower.startsWith("http_5")) return { variant: "error", label: formatStatusLabel(raw) ?? raw };
  if (lower.startsWith("http_"))
    return { variant: "secondary", label: formatStatusLabel(raw) ?? raw };
  return { variant: "secondary", label: raw };
}

function statusTextClassName(variant: StatusMeta["variant"]) {
  switch (variant) {
    case "success":
      return "text-success";
    case "warning":
      return "text-warning";
    case "error":
      return "text-error";
    case "default":
      return "text-info";
    default:
      return "text-base-content/70";
  }
}

interface InvocationRowViewModel {
  record: ApiInvocation;
  rowKey: string;
  recordId: number;
  meta: StatusMeta;
  statusLabel: string;
  livePhase: ApiInvocation["livePhase"];
  isInFlight: boolean;
  occurredTime: string;
  occurredDate: string;
  accountLabel: string;
  accountId: number | null;
  accountClickable: boolean;
  accountRoutingInProgress: boolean;
  proxyDisplayName: string;
  modelValue: string;
  modelHasMismatch: boolean;
  requestModelValue: string;
  responseModelValue: string;
  requestedServiceTierValue: string;
  serviceTierValue: string;
  billingServiceTierValue: string;
  fastIndicatorState: FastIndicatorState;
  costValue: string;
  inputTokensValue: string;
  cacheInputTokensValue: string;
  outputTokensValue: string;
  outputReasoningBreakdownValue: string;
  reasoningTokensValue: string;
  reasoningEffortValue: string;
  totalTokensValue: string;
  endpointValue: string;
  endpointDisplay: InvocationEndpointDisplay;
  imageIntentDisplay: InvocationImageIntentDisplay;
  errorMessage: string;
  collapsedErrorSummary: string;
  totalLatencyValue: string;
  firstResponseByteTotalValue: string;
  responseContentEncodingValue: string;
  detailNotice: string | null;
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>;
  timingPairs: Array<{ label: string; value: string }>;
}

export function InvocationTable({
  records,
  isLoading,
  error,
  emptyLabel,
  onOpenUpstreamAccount,
  scrollElement,
  showInvokeId = false,
  scrollTarget,
}: InvocationTableProps) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [isXlUp, setIsXlUp] = useState(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return false;
    return window.matchMedia("(min-width: 1280px)").matches;
  });
  const [isMdUp, setIsMdUp] = useState(() => {
    if (typeof window === "undefined") return false;
    if (typeof window.matchMedia === "function") {
      return window.matchMedia("(min-width: 768px)").matches || window.innerWidth >= 768;
    }
    return window.innerWidth >= 768;
  });
  const [containerElement, setContainerElement] = useState<HTMLDivElement | null>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  const [highlightedInvokeId, setHighlightedInvokeId] = useState<string | null>(null);
  const measureRefs = useRef(new Map<number, HTMLElement>());
  const handledScrollTargetVersionRef = useRef<number | null>(null);
  const highlightTimeoutRef = useRef<number | null>(null);
  const focusFrameRefs = useRef<number[]>([]);

  const toggleLabels = useMemo(() => {
    if (locale === "zh") {
      return {
        header: "详情",
        show: "展开详情",
        hide: "收起详情",
        expanded: "已展开",
        collapsed: "未展开",
      };
    }
    return {
      header: "Details",
      show: "Show details",
      hide: "Hide details",
      expanded: "Expanded",
      collapsed: "Collapsed",
    };
  }, [locale]);

  const openAccountDrawer = useCallback(
    (accountId: number | null, accountLabel: string) => {
      if (accountId == null) return;
      onOpenUpstreamAccount?.(accountId, accountLabel);
    },
    [onOpenUpstreamAccount],
  );

  const renderAccountValue = useCallback(
    (
      accountLabel: string,
      accountId: number | null,
      accountClickable: boolean,
      className?: string,
    ) => {
      if (!accountClickable || accountId == null) {
        return (
          <span
            className={cn(
              "inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap leading-none",
              className,
            )}
            title={accountLabel}
          >
            {accountLabel}
          </span>
        );
      }

      return (
        <button
          type="button"
          className={cn(
            "inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap appearance-none border-0 bg-transparent p-0 align-middle font-inherit leading-none text-center text-current no-underline shadow-none transition hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
            className,
          )}
          onClick={() => openAccountDrawer(accountId, accountLabel)}
          title={accountLabel}
        >
          {accountLabel}
        </button>
      );
    },
    [openAccountDrawer],
  );

  useEffect(() => {
    setExpandedId((current) => {
      if (current === null) return current;
      return records.some((record) => invocationStableKey(record) === current) ? current : null;
    });
  }, [records]);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mediaQuery = window.matchMedia("(min-width: 1280px)");
    const sync = () => {
      setIsXlUp(mediaQuery.matches);
    };

    sync();
    if (typeof mediaQuery.addEventListener === "function") {
      mediaQuery.addEventListener("change", sync);
      return () => {
        mediaQuery.removeEventListener("change", sync);
      };
    }

    mediaQuery.addListener(sync);
    return () => {
      mediaQuery.removeListener(sync);
    };
  }, []);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const mediaQuery =
      typeof window.matchMedia === "function" ? window.matchMedia("(min-width: 768px)") : null;
    const sync = () => {
      setIsMdUp((mediaQuery?.matches ?? false) || window.innerWidth >= 768);
    };

    sync();
    if (!mediaQuery) {
      window.addEventListener("resize", sync);
      return () => window.removeEventListener("resize", sync);
    }
    if (typeof mediaQuery.addEventListener === "function") {
      mediaQuery.addEventListener("change", sync);
      window.addEventListener("resize", sync);
      return () => {
        mediaQuery.removeEventListener("change", sync);
        window.removeEventListener("resize", sync);
      };
    }

    mediaQuery.addListener(sync);
    window.addEventListener("resize", sync);
    return () => {
      mediaQuery.removeListener(sync);
      window.removeEventListener("resize", sync);
    };
  }, []);

  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: "2-digit",
        day: "2-digit",
      }),
    [localeTag],
  );
  const timeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    [localeTag],
  );
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );

  const rows = useMemo<InvocationRowViewModel[]>(
    () =>
      records.map((record) => {
        const rowKey = invocationStableKey(record);
        const occurred = new Date(record.occurredAt);
        const displayStatus = resolveInvocationDisplayStatus(record);
        const normalizedStatus = (displayStatus || "unknown").toLowerCase();
        const meta = resolveStatusMeta(displayStatus);
        const livePhase = resolveInvocationLivePhase(record);
        const statusLabel = meta.labelKey
          ? t(meta.labelKey)
          : (meta.label ?? t("table.status.unknown"));
        const recordId = record.id;
        const isInFlight = normalizedStatus === "running" || normalizedStatus === "pending";
        const occurredValid = !Number.isNaN(occurred.getTime());
        const occurredTime = occurredValid ? timeFormatter.format(occurred) : record.occurredAt;
        const occurredDate = occurredValid ? dateFormatter.format(occurred) : FALLBACK_CELL;
        const detailView = buildInvocationDetailViewModel({
          record,
          normalizedStatus,
          t,
          locale,
          localeTag,
          nowMs,
          numberFormatter,
          currencyFormatter,
          renderAccountValue,
        });

        return {
          record,
          rowKey,
          recordId,
          meta,
          statusLabel,
          livePhase,
          isInFlight,
          occurredTime,
          occurredDate,
          ...detailView,
        };
      }),
    [
      records,
      currencyFormatter,
      dateFormatter,
      locale,
      localeTag,
      nowMs,
      numberFormatter,
      renderAccountValue,
      t,
      timeFormatter,
    ],
  );

  const hasInFlightRows = useMemo(() => rows.some((row) => row.isInFlight), [rows]);
  const expandedRecord = useMemo(
    () => rows.find((row) => row.rowKey === expandedId)?.record ?? null,
    [expandedId, rows],
  );
  const poolAttemptsState = useInvocationPoolAttempts(expandedRecord);
  const estimateRowSize = useCallback(
    (index: number) =>
      expandedId === rows[index]?.rowKey ? (isMdUp ? 320 : 430) : isMdUp ? 74 : 285,
    [expandedId, isMdUp, rows],
  );
  const measureVirtualItemElement = useCallback(
    (element: HTMLElement) => {
      const baseHeight = element.getBoundingClientRect().height;
      if (!isMdUp || element.tagName !== "TR") {
        return baseHeight;
      }

      const rowIndex = Number(element.dataset.index);
      if (!Number.isFinite(rowIndex)) {
        return baseHeight;
      }

      const row = rows[rowIndex];
      if (!row || expandedId !== row.rowKey) {
        return baseHeight;
      }

      const detailRow = element.nextElementSibling;
      if (detailRow?.tagName !== "TR") {
        return baseHeight;
      }

      return baseHeight + detailRow.getBoundingClientRect().height;
    },
    [expandedId, isMdUp, rows],
  );
  const elementVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollElement ?? null,
    estimateSize: estimateRowSize,
    measureElement: measureVirtualItemElement,
    overscan: 8,
    scrollMargin,
  });
  const windowVirtualizer = useWindowVirtualizer({
    count: rows.length,
    estimateSize: estimateRowSize,
    measureElement: measureVirtualItemElement,
    overscan: 8,
    scrollMargin,
  });
  const rowVirtualizer = scrollElement ? elementVirtualizer : windowVirtualizer;
  const scheduleMeasureElement = useCallback(
    (element: HTMLElement) => {
      if (typeof window === "undefined") {
        rowVirtualizer.measureElement(element);
        return;
      }
      window.requestAnimationFrame(() => {
        rowVirtualizer.measureElement(element);
      });
    },
    [rowVirtualizer],
  );
  const virtualRows = rowVirtualizer.getVirtualItems();
  const fallbackVirtualRows =
    virtualRows.length > 0
      ? virtualRows
      : rows.slice(0, Math.min(rows.length, 20)).map((_, index) => ({
          key: index,
          index,
          start: index * estimateRowSize(index),
          size: estimateRowSize(index),
          end: (index + 1) * estimateRowSize(index),
          lane: 0,
        }));
  const totalVirtualSize =
    virtualRows.length > 0
      ? rowVirtualizer.getTotalSize()
      : rows.reduce((sum, _, index) => sum + estimateRowSize(index), 0);

  useLayoutEffect(() => {
    const updateScrollMargin = () => {
      if (!containerElement || typeof window === "undefined") {
        setScrollMargin(0);
        return;
      }
      const containerRect = containerElement.getBoundingClientRect();
      const nextScrollMargin = scrollElement
        ? containerRect.top - scrollElement.getBoundingClientRect().top + scrollElement.scrollTop
        : containerRect.top + window.scrollY;
      setScrollMargin((current) =>
        Math.abs(current - nextScrollMargin) > 0.5 ? nextScrollMargin : current,
      );
    };

    updateScrollMargin();
    if (!containerElement || typeof window === "undefined") {
      return;
    }
    window.addEventListener("resize", updateScrollMargin);
    const scrollTarget = scrollElement ?? window;
    scrollTarget.addEventListener("scroll", updateScrollMargin, {
      passive: true,
    });
    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.removeEventListener("resize", updateScrollMargin);
        scrollTarget.removeEventListener("scroll", updateScrollMargin);
      };
    }
    const observer = new ResizeObserver(updateScrollMargin);
    observer.observe(containerElement);
    if (scrollElement) observer.observe(scrollElement);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", updateScrollMargin);
      scrollTarget.removeEventListener("scroll", updateScrollMargin);
    };
  }, [containerElement, scrollElement]);

  useLayoutEffect(() => {
    const element = expandedId
      ? measureRefs.current.get(rows.findIndex((row) => row.rowKey === expandedId))
      : null;
    if (element) rowVirtualizer.measureElement(element);
  }, [expandedId, rowVirtualizer, rows]);

  useLayoutEffect(() => {
    if (!scrollTarget || handledScrollTargetVersionRef.current === scrollTarget.version) {
      return;
    }
    const targetIndex = rows.findIndex((row) => row.record.invokeId === scrollTarget.invokeId);
    if (targetIndex < 0) return;

    handledScrollTargetVersionRef.current = scrollTarget.version;
    rowVirtualizer.scrollToIndex(targetIndex, { align: "center" });
    setHighlightedInvokeId(scrollTarget.invokeId);

    focusFrameRefs.current.forEach((frame) => {
      window.cancelAnimationFrame(frame);
    });
    focusFrameRefs.current = [];
    const firstFrame = window.requestAnimationFrame(() => {
      const secondFrame = window.requestAnimationFrame(() => {
        measureRefs.current.get(targetIndex)?.focus({ preventScroll: true });
      });
      focusFrameRefs.current.push(secondFrame);
    });
    focusFrameRefs.current.push(firstFrame);
    if (highlightTimeoutRef.current != null) {
      window.clearTimeout(highlightTimeoutRef.current);
    }
    highlightTimeoutRef.current = window.setTimeout(() => {
      setHighlightedInvokeId((current) => (current === scrollTarget.invokeId ? null : current));
      highlightTimeoutRef.current = null;
    }, 2_000);
  }, [rowVirtualizer, rows, scrollTarget]);

  useEffect(
    () => () => {
      focusFrameRefs.current.forEach((frame) => {
        window.cancelAnimationFrame(frame);
      });
      if (highlightTimeoutRef.current != null) {
        window.clearTimeout(highlightTimeoutRef.current);
      }
    },
    [],
  );

  useLayoutEffect(() => {
    if (!highlightedInvokeId) return;
    const targetIndex = rows.findIndex((row) => row.record.invokeId === highlightedInvokeId);
    if (targetIndex < 0) return;
    const frame = window.requestAnimationFrame(() => {
      measureRefs.current.get(targetIndex)?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [highlightedInvokeId, rows]);

  useEffect(() => {
    if (!hasInFlightRows) return;
    setNowMs(Date.now());
    const id = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(id);
  }, [hasInFlightRows]);

  if (error) {
    return (
      <ListBodyState
        variant="error"
        title={t("table.loadError", { error })}
        testId="invocation-table-error"
      />
    );
  }

  if (isLoading) {
    return (
      <ListBodyState
        variant="loading"
        title={t("table.loadingRecordsAria")}
        testId="invocation-table-loading"
      />
    );
  }

  if (records.length === 0) {
    return (
      <ListBodyState
        variant="empty"
        title={emptyLabel ?? t("table.noRecords")}
        testId="invocation-table-empty"
      />
    );
  }

  const firstVirtualRow = fallbackVirtualRows[0] ?? null;
  const lastVirtualRow = fallbackVirtualRows[fallbackVirtualRows.length - 1] ?? null;
  const paddingTop = firstVirtualRow ? Math.max(0, firstVirtualRow.start - scrollMargin) : 0;
  const paddingBottom = lastVirtualRow
    ? Math.max(0, totalVirtualSize - (lastVirtualRow.end - scrollMargin))
    : 0;

  if (!isMdUp) {
    return (
      <div className="space-y-3" ref={setContainerElement}>
        <div className="space-y-3" data-testid="invocation-list">
          {paddingTop > 0 ? <div aria-hidden="true" style={{ height: paddingTop }} /> : null}
          {fallbackVirtualRows.map((virtualRow) => {
            const row = rows[virtualRow.index];
            if (!row) return null;
            const listDetailId = `invocation-list-details-${invocationStableDomKey(row.rowKey)}`;
            const isExpanded = expandedId === row.rowKey;
            const isHighlighted = highlightedInvokeId === row.record.invokeId;
            const handleToggle = () => {
              setExpandedId((current) => (current === row.rowKey ? null : row.rowKey));
            };

            return (
              <article
                key={`mobile-${row.rowKey}`}
                ref={(node) => {
                  if (node) {
                    if (measureRefs.current.get(virtualRow.index) !== node) {
                      measureRefs.current.set(virtualRow.index, node);
                      scheduleMeasureElement(node);
                    }
                  } else {
                    measureRefs.current.delete(virtualRow.index);
                  }
                }}
                data-index={virtualRow.index}
                data-invoke-id={row.record.invokeId ?? undefined}
                data-testid="invocation-list-item"
                tabIndex={isHighlighted ? -1 : undefined}
                aria-current={isHighlighted ? "true" : undefined}
                className={cn(
                  "rounded-lg border border-base-300/70 px-3 py-3 outline-none transition-colors motion-reduce:transition-none",
                  virtualRow.index % 2 === 0 ? "bg-base-100/40" : "bg-base-200/24",
                  isHighlighted && "border-primary/55 bg-primary/10",
                )}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-semibold">{row.occurredTime}</div>
                    <div className="truncate text-xs text-base-content/65">{row.occurredDate}</div>
                    {showInvokeId && row.record.invokeId ? (
                      <FittedInvocationId
                        invokeId={row.record.invokeId}
                        className="mt-1 font-mono text-info select-text"
                      />
                    ) : null}
                  </div>
                  <button
                    type="button"
                    className="inline-flex h-8 w-8 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                    onClick={handleToggle}
                    aria-expanded={isExpanded}
                    aria-controls={listDetailId}
                    aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                  >
                    <AppIcon
                      name={isExpanded ? "chevron-down" : "chevron-right"}
                      className="h-5 w-5"
                      aria-hidden
                    />
                    <span className="sr-only">
                      {isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}
                    </span>
                  </button>
                </div>

                <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
                  {row.livePhase ? (
                    <InvocationPhaseBadge
                      phase={row.livePhase}
                      appearance="inline"
                      motion="dynamic"
                    />
                  ) : (
                    <Badge variant={row.meta.variant}>{row.statusLabel}</Badge>
                  )}
                  <div className="min-w-0 flex-1">
                    <div data-testid="invocation-account-name">
                      {renderAccountValue(
                        row.accountLabel,
                        row.accountId,
                        row.accountClickable,
                        cn(
                          "text-xs font-medium text-base-content",
                          row.accountRoutingInProgress &&
                            INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
                        ),
                      )}
                    </div>
                    <div
                      className="min-w-0 truncate text-[11px] text-base-content/70"
                      title={row.proxyDisplayName}
                      data-testid="invocation-proxy-name"
                    >
                      {row.proxyDisplayName}
                    </div>
                  </div>
                </div>

                <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs font-mono text-base-content/70">
                  <span
                    title={row.totalLatencyValue}
                  >{`${t("table.column.totalLatencyShort")} ${row.totalLatencyValue}`}</span>
                  <span
                    title={row.firstResponseByteTotalValue}
                  >{`${t("table.column.firstResponseByteTotalShort")} ${row.firstResponseByteTotalValue}`}</span>
                  <span
                    title={row.responseContentEncodingValue}
                  >{`${t("table.column.httpCompressionShort")} ${row.responseContentEncodingValue}`}</span>
                </div>

                <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                  <dt className="text-base-content/65">{t("table.column.model")}</dt>
                  <dd className="min-w-0">
                    {row.modelHasMismatch ? (
                      renderInvocationModelRoutingSummary({
                        requestModelValue: row.requestModelValue,
                        responseModelValue: row.responseModelValue,
                        hasMismatch: true,
                        t,
                        adornments: (
                          <>
                            {renderInvocationTransportBadge(row.record)}
                            {renderFastIndicator(row.fastIndicatorState, t)}
                          </>
                        ),
                      })
                    ) : (
                      <div
                        className="flex items-center justify-end gap-1 text-right"
                        title={row.modelValue}
                      >
                        {renderInvocationModelBadge(row.modelValue, {
                          t,
                          hasMismatch: false,
                          textClassName: "text-right",
                          testId: "invocation-table-model",
                        })}
                        {renderInvocationTransportBadge(row.record)}
                        {renderFastIndicator(row.fastIndicatorState, t)}
                      </div>
                    )}
                  </dd>
                  <dt className="text-base-content/65">{t("table.column.costUsd")}</dt>
                  <dd className="truncate text-right font-mono">{row.costValue}</dd>
                  <dt className="text-base-content/65">{t("table.column.inputTokens")}</dt>
                  <dd className="truncate text-right font-mono">{row.inputTokensValue}</dd>
                  <dt className="text-base-content/65">{t("table.column.cacheInputTokens")}</dt>
                  <dd className="truncate text-right font-mono">{row.cacheInputTokensValue}</dd>
                  <dt className="text-base-content/65">{t("table.column.outputTokens")}</dt>
                  <dd className="text-right">
                    <div className="flex flex-col items-end gap-0.5 leading-tight">
                      <span className="truncate font-mono">{row.outputTokensValue}</span>
                      <span
                        className="truncate text-[11px] text-base-content/70"
                        title={`${t("table.details.reasoningTokens")}: ${row.reasoningTokensValue}`}
                      >
                        {row.outputReasoningBreakdownValue}
                      </span>
                    </div>
                  </dd>
                  <dt className="text-base-content/65">{t("table.column.totalTokens")}</dt>
                  <dd className="truncate text-right font-mono">{row.totalTokensValue}</dd>
                  <dt className="text-base-content/65">{t("table.column.reasoningEffort")}</dt>
                  <dd className="flex justify-end">
                    {renderReasoningEffortBadge(row.reasoningEffortValue)}
                  </dd>
                </dl>

                <div className="mt-3 space-y-1 border-t border-base-300/65 pt-2">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t("table.details.endpoint")}
                  </div>
                  <div className="flex min-w-0 flex-wrap items-center gap-1">
                    {renderEndpointSummary(row.endpointDisplay, t, "text-xs")}
                    {renderImageIntentBadge(
                      row.imageIntentDisplay,
                      t,
                      "h-5 border-transparent bg-base-100/70 px-2 text-[10px] shadow-none",
                    )}
                  </div>
                  <div className="truncate text-xs" title={row.collapsedErrorSummary || undefined}>
                    {row.collapsedErrorSummary || FALLBACK_CELL}
                  </div>
                </div>

                {isExpanded && (
                  <div className="mt-3 rounded-lg border border-base-300/70 bg-base-200/58">
                    <InvocationExpandedDetails
                      record={row.record}
                      detailId={listDetailId}
                      detailPairs={row.detailPairs}
                      timingPairs={row.timingPairs}
                      errorMessage={row.errorMessage}
                      detailNotice={row.detailNotice}
                      size="compact"
                      poolAttemptsState={poolAttemptsState}
                      focusedAttemptId={isHighlighted ? (scrollTarget?.attemptId ?? null) : null}
                      t={t}
                    />
                  </div>
                )}
              </article>
            );
          })}
          {paddingBottom > 0 ? <div aria-hidden="true" style={{ height: paddingBottom }} /> : null}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-3" ref={setContainerElement}>
      <div>
        <div
          className="overflow-x-hidden rounded-xl border border-base-300/70 bg-base-100/52 backdrop-blur"
          data-testid="invocation-table-scroll"
        >
          <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
            <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
              <tr>
                <th
                  className={cn(
                    "px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:px-3",
                    showInvokeId ? "w-[20%] xl:w-[16%]" : "w-[11%] xl:w-[10%]",
                  )}
                >
                  {t("table.column.time")}
                </th>
                <th
                  className={cn(
                    "px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:px-3",
                    showInvokeId ? "w-[16%] xl:w-[15%]" : "w-[18%] xl:w-[15%]",
                  )}
                >
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.account")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.column.proxy")}
                    </span>
                  </div>
                </th>
                <th
                  className={cn(
                    "px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:px-3",
                    showInvokeId ? "w-[10%] xl:w-[9%]" : "w-[13%] xl:w-[12%]",
                  )}
                >
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.latency")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.column.firstResponseByteTotalCompression")}
                    </span>
                  </div>
                </th>
                <th
                  className={cn(
                    "px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3",
                    showInvokeId ? "w-[15%] xl:w-[15%]" : "w-[17%] xl:w-[14%]",
                  )}
                >
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.model")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.column.costUsd")}
                    </span>
                  </div>
                </th>
                <th
                  className={cn(
                    "px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3",
                    showInvokeId ? "w-[11%] xl:w-[11%]" : "w-[16%] xl:w-[14%]",
                  )}
                >
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.inputTokens")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.column.cacheInputTokens")}
                    </span>
                  </div>
                </th>
                <th
                  className={cn(
                    "px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3",
                    showInvokeId ? "w-[8%] xl:w-[8%]" : "w-[10%] xl:w-[10%]",
                  )}
                >
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.outputTokens")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.details.reasoningTokens")}
                    </span>
                  </div>
                </th>
                <th className="w-[12%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[11%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.totalTokens")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.column.reasoningEffort")}
                    </span>
                  </div>
                </th>
                <th className="hidden w-[10%] px-2 py-2.5 text-left font-semibold xl:table-cell xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t("table.column.error")}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t("table.details.endpoint")}
                    </span>
                  </div>
                </th>
                <th
                  className={cn(
                    "px-2 py-2.5 text-right xl:px-3",
                    showInvokeId ? "w-[8%] xl:w-[5%]" : "w-[9%] xl:w-[4%]",
                  )}
                >
                  <span className="sr-only">{toggleLabels.header}</span>
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-base-300/65">
              {paddingTop > 0 ? (
                <tr>
                  <td colSpan={isXlUp ? 9 : 8} style={{ height: paddingTop, padding: 0 }} />
                </tr>
              ) : null}
              {fallbackVirtualRows.map((virtualRow) => {
                const row = rows[virtualRow.index];
                if (!row) return null;
                const tableDetailId = `invocation-table-details-${invocationStableDomKey(row.rowKey)}`;
                const isExpanded = expandedId === row.rowKey;
                const isHighlighted = highlightedInvokeId === row.record.invokeId;
                const handleToggle = () => {
                  setExpandedId((current) => (current === row.rowKey ? null : row.rowKey));
                };

                return (
                  <Fragment key={row.rowKey}>
                    <tr
                      ref={(node) => {
                        if (node) {
                          if (measureRefs.current.get(virtualRow.index) !== node) {
                            measureRefs.current.set(virtualRow.index, node);
                            scheduleMeasureElement(node);
                          }
                        } else {
                          measureRefs.current.delete(virtualRow.index);
                        }
                      }}
                      data-index={virtualRow.index}
                      data-invoke-id={row.record.invokeId ?? undefined}
                      tabIndex={isHighlighted ? -1 : undefined}
                      aria-current={isHighlighted ? "true" : undefined}
                      className={cn(
                        "outline-none transition-colors hover:bg-primary/6 motion-reduce:transition-none",
                        virtualRow.index % 2 === 0 ? "bg-base-100/38" : "bg-base-200/22",
                        isHighlighted && "bg-primary/10 ring-1 ring-inset ring-primary/45",
                      )}
                    >
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <span className="truncate whitespace-nowrap font-medium">
                            {row.occurredTime}
                          </span>
                          <span className="truncate whitespace-nowrap text-base-content/70">
                            {row.occurredDate}
                          </span>
                          {showInvokeId && row.record.invokeId ? (
                            <FittedInvocationId
                              invokeId={row.record.invokeId}
                              className="font-mono text-info select-text"
                            />
                          ) : null}
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-center justify-center gap-1 leading-tight text-center">
                          {row.livePhase ? (
                            <InvocationPhaseBadge
                              phase={row.livePhase}
                              appearance="inline"
                              motion="dynamic"
                              className="justify-center text-[11px] font-semibold"
                            />
                          ) : (
                            <span
                              className={cn(
                                "block max-w-full truncate whitespace-nowrap text-center text-[11px] font-semibold leading-none",
                                statusTextClassName(row.meta.variant),
                              )}
                              data-testid="invocation-proxy-badge"
                              title={row.statusLabel}
                            >
                              {row.statusLabel}
                            </span>
                          )}
                          <div className="mx-auto flex w-fit max-w-full items-center justify-center overflow-hidden text-center text-[11px] font-semibold leading-none text-base-content">
                            <span
                              className="inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap leading-none"
                              data-testid="invocation-account-name"
                            >
                              {renderAccountValue(
                                row.accountLabel,
                                row.accountId,
                                row.accountClickable,
                                row.accountRoutingInProgress
                                  ? INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME
                                  : undefined,
                              )}
                            </span>
                          </div>
                          <span
                            className="block w-full truncate whitespace-nowrap text-center text-[11px] text-base-content/70"
                            title={row.proxyDisplayName}
                            data-testid="invocation-proxy-name"
                          >
                            {row.proxyDisplayName}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <span
                            className="truncate whitespace-nowrap font-mono tabular-nums"
                            title={row.totalLatencyValue}
                          >
                            {row.totalLatencyValue}
                          </span>
                          <span
                            className="truncate whitespace-nowrap text-[11px] text-base-content/70"
                            title={`${row.firstResponseByteTotalValue} · ${row.responseContentEncodingValue}`}
                          >
                            {`${row.firstResponseByteTotalValue} · ${row.responseContentEncodingValue}`}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <div className="flex w-full items-center justify-end gap-1">
                            {renderInvocationModelBadge(row.modelValue, {
                              t,
                              hasMismatch: row.modelHasMismatch,
                              textClassName: "whitespace-nowrap text-base-content/85",
                              testId: "invocation-table-model",
                            })}
                            {renderInvocationTransportBadge(row.record)}
                            {renderFastIndicator(row.fastIndicatorState, t)}
                          </div>
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                            {row.costValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums">
                            {row.inputTokensValue}
                          </span>
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                            {row.cacheInputTokensValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="block w-full truncate whitespace-nowrap font-mono tabular-nums">
                            {row.outputTokensValue}
                          </span>
                          <span
                            className="block w-full truncate whitespace-nowrap text-[11px] text-base-content/70"
                            title={`${t("table.details.reasoningTokens")}: ${row.reasoningTokensValue}`}
                          >
                            {row.outputReasoningBreakdownValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="block w-full truncate whitespace-nowrap font-mono tabular-nums">
                            {row.totalTokensValue}
                          </span>
                          <div className="flex w-full justify-end">
                            {renderReasoningEffortBadge(row.reasoningEffortValue)}
                          </div>
                        </div>
                      </td>
                      <td className="hidden min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:table-cell xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <div className="flex min-w-0 flex-wrap items-center gap-1">
                            {renderEndpointSummary(row.endpointDisplay, t)}
                            {renderImageIntentBadge(
                              row.imageIntentDisplay,
                              t,
                              "h-5 border-transparent bg-base-100/70 px-2 text-[10px] shadow-none",
                            )}
                          </div>
                          <span
                            className="block truncate whitespace-nowrap"
                            title={row.collapsedErrorSummary || undefined}
                          >
                            {row.collapsedErrorSummary || FALLBACK_CELL}
                          </span>
                        </div>
                      </td>
                      <td className="border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <button
                          type="button"
                          className="inline-flex items-center justify-end gap-1 text-lg leading-none text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                          onClick={handleToggle}
                          aria-expanded={isExpanded}
                          aria-controls={tableDetailId}
                          aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                        >
                          <AppIcon
                            name={isExpanded ? "chevron-down" : "chevron-right"}
                            className="h-4 w-4"
                            aria-hidden
                          />
                          <span className="sr-only">
                            {isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}
                          </span>
                        </button>
                      </td>
                    </tr>
                    {isExpanded && (
                      <tr className="bg-base-200/68">
                        <td
                          colSpan={isXlUp ? 9 : 8}
                          className="border-t border-base-300/65 px-2 py-2.5 xl:px-3"
                        >
                          <InvocationExpandedDetails
                            record={row.record}
                            detailId={tableDetailId}
                            detailPairs={row.detailPairs}
                            timingPairs={row.timingPairs}
                            errorMessage={row.errorMessage}
                            detailNotice={row.detailNotice}
                            size="default"
                            poolAttemptsState={poolAttemptsState}
                            focusedAttemptId={
                              isHighlighted ? (scrollTarget?.attemptId ?? null) : null
                            }
                            t={t}
                          />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
              {paddingBottom > 0 ? (
                <tr>
                  <td colSpan={isXlUp ? 9 : 8} style={{ height: paddingBottom, padding: 0 }} />
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
