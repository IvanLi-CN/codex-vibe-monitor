import { useVirtualizer, useWindowVirtualizer } from "@tanstack/react-virtual";
import {
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Chip, type SemanticChipTone } from "../../components/ui/chip";
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
import { InvocationPhaseChip } from "./InvocationPhaseChip";
import { InvocationWorkflowDetailPanel } from "./InvocationWorkflowDetailPanel";
import {
  buildInvocationDetailViewModel,
  FALLBACK_CELL,
  INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
  renderEndpointSummary,
  renderFastIndicator,
  renderImageIntentChip,
  renderInvocationModelChip,
  renderReasoningEffortChip,
} from "./invocation-details-shared";
import { renderInvocationTransportChip } from "./invocation-transport-chip";

interface InvocationTableProps {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel?: string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  scrollElement?: HTMLElement | null;
  scrollTarget?: { invokeId: string; attemptId?: string | null; version: number } | null;
}

type StatusMeta = {
  variant: Exclude<SemanticChipTone, "neutral" | "accent" | "info">;
  labelKey?: TranslationKey;
  label?: string;
};

const STATUS_META: Record<string, { variant: StatusMeta["variant"]; labelKey: TranslationKey }> = {
  success: { variant: "success", labelKey: "table.status.success" },
  completed: { variant: "success", labelKey: "table.status.success" },
  warning_success: { variant: "warning", labelKey: "table.status.warningSuccess" },
  failed: { variant: "error", labelKey: "table.status.failed" },
  interrupted: { variant: "error", labelKey: "table.status.interrupted" },
  running: { variant: "primary", labelKey: "table.status.running" },
  pending: { variant: "warning", labelKey: "table.status.pending" },
};

const INVOCATION_ID_BASE_FONT_SIZE_PX = 10;
const INVOCATION_CARD_GAP_PX = 12;

function hasMeasuredDurationMs(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function formatCompactLatencySeconds(
  value: number | null | undefined,
  localeTag: string,
  fallback: string,
) {
  if (!hasMeasuredDurationMs(value)) return fallback;
  const roundedSeconds = Math.round((value / 1000) * 10) / 10;
  const maximumFractionDigits = roundedSeconds >= 100 ? 0 : 1;
  return `${roundedSeconds.toLocaleString(localeTag, {
    useGrouping: false,
    minimumFractionDigits: 0,
    maximumFractionDigits,
  })} s`;
}

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

interface InvocationRowViewModel {
  record: ApiInvocation;
  rowKey: string;
  recordId: number;
  meta: StatusMeta;
  statusLabel: string;
  livePhase: ApiInvocation["livePhase"];
  occurredTime: string;
  occurredDate: string;
  accountLabel: string;
  accountId: number | null;
  accountClickable: boolean;
  accountRoutingInProgress: boolean;
  accountPlanType: string | null;
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
  cacheWriteTokensValue: string;
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
  responseDurationValue: string;
  firstTokenValue: string;
  requestCompressionAlgorithmValue: string;
  responseContentEncodingValue: string;
  detailNotice: string | null;
  detailPairs: Array<{ key: string; label: string; value: ReactNode }>;
  timingPairs: Array<{ label: string; value: string }>;
}

export function InvocationCardList({
  records,
  isLoading,
  error,
  emptyLabel,
  onOpenUpstreamAccount,
  scrollElement,
  scrollTarget,
}: InvocationTableProps) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const [expandedId, setExpandedId] = useState<string | null>(null);
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

  const scheduleHighlightClear = useCallback(() => {
    if (!highlightedInvokeId || typeof window === "undefined") return;
    if (highlightTimeoutRef.current != null) {
      window.clearTimeout(highlightTimeoutRef.current);
    }
    const invokeId = highlightedInvokeId;
    highlightTimeoutRef.current = window.setTimeout(() => {
      setHighlightedInvokeId((current) => (current === invokeId ? null : current));
      highlightTimeoutRef.current = null;
    }, 1_500);
  }, [highlightedInvokeId]);

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
          onClick={(event) => {
            event.stopPropagation();
            openAccountDrawer(accountId, accountLabel);
          }}
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
        const occurredValid = !Number.isNaN(occurred.getTime());
        const occurredTime = occurredValid ? timeFormatter.format(occurred) : record.occurredAt;
        const occurredDate = occurredValid ? dateFormatter.format(occurred) : FALLBACK_CELL;
        const detailView = buildInvocationDetailViewModel({
          record,
          normalizedStatus,
          t,
          locale,
          localeTag,
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
      numberFormatter,
      renderAccountValue,
      t,
      timeFormatter,
    ],
  );

  const listSummary = useMemo(() => {
    const successful = rows.filter((row) => row.meta.variant === "success").length;
    const failed = rows.filter((row) => row.meta.variant === "error").length;
    const running = rows.filter((row) => row.livePhase != null).length;
    const firstTokenSamples = rows
      .map((row) => row.record.firstTokenMs)
      .filter(hasMeasuredDurationMs);
    const responseSamples = rows
      .map((row) => row.record.tUpstreamStreamMs)
      .filter(hasMeasuredDurationMs);
    const totalCost = rows.reduce(
      (sum, row) => sum + (typeof row.record.cost === "number" ? row.record.cost : 0),
      0,
    );
    const totalTokens = rows.reduce(
      (sum, row) => sum + (typeof row.record.totalTokens === "number" ? row.record.totalTokens : 0),
      0,
    );
    const cacheRead = rows.reduce(
      (sum, row) => sum + Math.max(0, row.record.cacheInputTokens ?? 0),
      0,
    );
    const cacheWrite = rows.reduce(
      (sum, row) => sum + Math.max(0, row.record.cacheWriteTokens ?? 0),
      0,
    );
    const cacheHitRate =
      cacheRead + cacheWrite > 0 ? Math.round((cacheRead / (cacheRead + cacheWrite)) * 100) : null;

    return {
      successful,
      failed,
      running,
      totalCost,
      totalTokens,
      cacheHitRate,
      averageFirstToken:
        firstTokenSamples.length > 0
          ? firstTokenSamples.reduce((sum, value) => sum + value, 0) / firstTokenSamples.length
          : null,
      averageResponse:
        responseSamples.length > 0
          ? responseSamples.reduce((sum, value) => sum + value, 0) / responseSamples.length
          : null,
    };
  }, [rows]);

  const estimateRowSize = useCallback(
    (index: number) => {
      const baseSize =
        expandedId === rows[index]?.rowKey ? (isMdUp ? 360 : 520) : isMdUp ? 106 : 132;
      return baseSize + (index < rows.length - 1 ? INVOCATION_CARD_GAP_PX : 0);
    },
    [expandedId, isMdUp, rows],
  );
  const measureVirtualItemElement = useCallback(
    (element: HTMLElement) => {
      const baseHeight = element.getBoundingClientRect().height;
      const rowIndex = Number(element.dataset.index);
      return (
        baseHeight +
        (Number.isFinite(rowIndex) && rowIndex < rows.length - 1 ? INVOCATION_CARD_GAP_PX : 0)
      );
    },
    [rows],
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
    setExpandedId(rows[targetIndex]?.rowKey ?? null);
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

  const formatSummaryDuration = (value: number | null) => {
    return formatCompactLatencySeconds(value, localeTag, FALLBACK_CELL);
  };

  const formatSummaryNumber = (value: number) => numberFormatter.format(value);

  const summaryTitle =
    locale === "zh" ? `最近 ${rows.length} 条调用` : `${rows.length} most recent calls`;
  const summaryMetrics = [
    {
      key: "ttft",
      label: locale === "zh" ? "TTFT" : "TTFT",
      value: formatSummaryDuration(listSummary.averageFirstToken),
      detail: formatSummaryDuration(listSummary.averageResponse),
      icon: "timer-outline" as const,
      tone: "text-success",
    },
    {
      key: "requests",
      label: locale === "zh" ? "请求数" : "Requests",
      value: formatSummaryNumber(rows.length),
      detail: `${listSummary.successful} ${locale === "zh" ? "成功" : "ok"} · ${listSummary.failed} ${locale === "zh" ? "失败" : "failed"}`,
      icon: "counter" as const,
      tone: "text-success",
    },
    {
      key: "cost",
      label: locale === "zh" ? "成本" : "Cost",
      value: listSummary.totalCost > 0 ? `$${listSummary.totalCost.toFixed(4)}` : FALLBACK_CELL,
      detail: `${listSummary.running} ${locale === "zh" ? "进行中" : "in flight"}`,
      icon: "currency-usd" as const,
      tone: "text-warning",
    },
    {
      key: "tokens",
      label: locale === "zh" ? "TOKEN" : "Tokens",
      value: formatSummaryNumber(listSummary.totalTokens),
      detail:
        listSummary.cacheHitRate == null
          ? FALLBACK_CELL
          : `${listSummary.cacheHitRate}% ${locale === "zh" ? "命中" : "cache hit"}`,
      icon: "database-outline" as const,
      tone: "text-success",
    },
  ];

  const renderCard = (row: InvocationRowViewModel, virtualIndex: number) => {
    const cardDetailId = `invocation-card-details-${invocationStableDomKey(row.rowKey)}`;
    const isExpanded = expandedId === row.rowKey;
    const isHighlighted = highlightedInvokeId === row.record.invokeId;
    const handleToggle = () => {
      setExpandedId((current) => (current === row.rowKey ? null : row.rowKey));
    };
    const isInsideInvocationDetail = (target: EventTarget | null) =>
      target instanceof Element && target.closest("[data-invocation-detail]") != null;
    const handleCardClick = (event: MouseEvent<HTMLElement>) => {
      if (event.target instanceof Node && !event.currentTarget.contains(event.target)) return;
      if (isInsideInvocationDetail(event.target)) return;
      handleToggle();
    };
    const handleKeyDown = (event: KeyboardEvent<HTMLElement>) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      if (event.target !== event.currentTarget) return;
      if (isInsideInvocationDetail(event.target)) return;
      event.preventDefault();
      handleToggle();
    };
    const firstTokenValue = formatCompactLatencySeconds(row.record.firstTokenMs, localeTag, "--");
    const responseDurationValue = formatCompactLatencySeconds(
      row.record.tUpstreamStreamMs,
      localeTag,
      "--",
    );
    const hasMeasuredFirstToken = hasMeasuredDurationMs(row.record.firstTokenMs);
    const cacheReadTokens = Math.max(0, row.record.cacheInputTokens ?? 0);
    const cacheWriteTokens = Math.max(
      0,
      row.record.cacheWriteTokens ?? Math.max(0, (row.record.inputTokens ?? 0) - cacheReadTokens),
    );
    const outputTokens = Math.max(0, row.record.outputTokens ?? 0);
    const cacheHitDenominator = cacheWriteTokens + cacheReadTokens + outputTokens;
    const cacheHitRate =
      cacheHitDenominator > 0
        ? `${Math.round((cacheReadTokens / cacheHitDenominator) * 100)}%`
        : FALLBACK_CELL;
    const shortInvocationId = row.record.invokeId.slice(-6).toUpperCase();
    const modelLabel = row.modelHasMismatch
      ? `${row.requestModelValue} → ${row.responseModelValue}`
      : row.modelValue;
    const secondaryChips = [
      row.reasoningEffortValue !== FALLBACK_CELL ? (
        <span key="reasoning" className="inline-flex items-center">
          {renderReasoningEffortChip(row.reasoningEffortValue)}
        </span>
      ) : null,
      row.fastIndicatorState !== "none" ? (
        <span key="fast" className="inline-flex items-center">
          {renderFastIndicator(row.fastIndicatorState, t)}
        </span>
      ) : null,
    ].filter(Boolean);

    return (
      <article
        key={row.rowKey}
        ref={(node) => {
          if (node) {
            if (measureRefs.current.get(virtualIndex) !== node) {
              measureRefs.current.set(virtualIndex, node);
              scheduleMeasureElement(node);
            }
          } else {
            measureRefs.current.delete(virtualIndex);
          }
        }}
        data-index={virtualIndex}
        data-invoke-id={row.record.invokeId ?? undefined}
        data-testid="invocation-card"
        tabIndex={isHighlighted ? -1 : 0}
        aria-current={isHighlighted ? "true" : undefined}
        aria-label={`${row.statusLabel} ${row.record.invokeId}`}
        data-expanded={isExpanded}
        onClick={handleCardClick}
        onKeyDown={handleKeyDown}
        className={cn(
          "surface-card min-w-0 overflow-hidden rounded-lg px-3 py-2.5 text-left outline-none transition-colors hover:border-primary/45 hover:bg-primary/5 focus-visible:border-primary/65 focus-visible:ring-2 focus-visible:ring-primary/35 motion-reduce:transition-none md:px-3.5 md:py-2.5",
          virtualIndex < rows.length - 1 && "mb-3",
          isHighlighted && "border-primary/55 bg-primary/10",
        )}
      >
        <button
          type="button"
          tabIndex={-1}
          className="sr-only"
          aria-expanded={isExpanded}
          aria-controls={cardDetailId}
          aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
          onClick={(event) => {
            event.stopPropagation();
            handleToggle();
          }}
          data-testid="invocation-card-toggle"
        >
          {isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}
        </button>
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5">
          <Chip
            tone="secondary"
            className="px-2 py-0.5 font-mono text-[10px] font-semibold tracking-[0.04em]"
            title={row.record.invokeId}
          >
            {shortInvocationId}
          </Chip>
          <FittedInvocationId
            invokeId={row.record.invokeId}
            className="max-w-[9rem] flex-none font-mono text-[11px] font-semibold text-base-content/88 select-text sm:max-w-[13rem]"
          />
          {row.livePhase ? (
            <InvocationPhaseChip phase={row.livePhase} appearance="inline" motion="dynamic" />
          ) : (
            <Chip
              tone={row.meta.variant}
              className="px-1.5 py-0 text-[10px]"
              data-testid="invocation-proxy-badge"
            >
              {row.statusLabel}
            </Chip>
          )}
          {renderInvocationTransportChip(row.record)}
          <span
            className="min-w-0 max-w-[10rem] truncate text-[11px] text-base-content/68"
            title={row.endpointValue}
          >
            <span className="inline-flex min-w-0 items-center gap-1">
              {renderEndpointSummary(row.endpointDisplay, t, "text-[11px]")}
              {renderImageIntentChip(row.imageIntentDisplay, t, "h-5 px-1.5 text-[10px]")}
            </span>
          </span>
          <div className="ml-auto flex shrink-0 items-center gap-x-3 font-mono text-[11px] tabular-nums">
            <span
              className={cn(
                "inline-flex items-center gap-1",
                hasMeasuredFirstToken ? "text-success" : "text-base-content/58",
              )}
              data-testid="invocation-card-ttft"
              title={`${t("table.column.firstTokenShort")}: ${firstTokenValue}`}
            >
              <AppIcon name="timer-outline" className="h-3.5 w-3.5" aria-hidden />
              {t("table.column.firstTokenShort")} {firstTokenValue}
            </span>
            <span
              className="inline-flex items-center gap-1 text-info"
              data-testid="invocation-card-response"
              title={`${t("table.column.responseDurationShort")}: ${responseDurationValue}`}
            >
              <AppIcon name="timer-refresh-outline" className="h-3.5 w-3.5" aria-hidden />
              {t("table.column.responseDurationShort")} {responseDurationValue}
            </span>
          </div>
          <button
            type="button"
            className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
            onClick={(event) => {
              event.stopPropagation();
              handleToggle();
            }}
            aria-expanded={isExpanded}
            aria-controls={cardDetailId}
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

        <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1 text-[11px] text-base-content/68">
          <span className="font-mono tabular-nums text-base-content/82">
            {row.occurredDate} {row.occurredTime}
          </span>
          <span className="text-base-content/35">·</span>
          <span data-testid="invocation-account-name" className="min-w-0 max-w-[9rem] truncate">
            {renderAccountValue(
              row.accountLabel,
              row.accountId,
              row.accountClickable,
              cn(
                "max-w-full truncate font-medium text-base-content/82",
                row.accountRoutingInProgress && INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
              ),
            )}
          </span>
          {row.accountPlanType ? (
            <span className="text-base-content/55">{row.accountPlanType}</span>
          ) : null}
          <span className="text-base-content/35">·</span>
          <span
            className="min-w-0 max-w-[10rem] truncate"
            title={row.proxyDisplayName}
            data-testid="invocation-proxy-name"
          >
            {row.proxyDisplayName}
          </span>
          <span className="text-base-content/35">·</span>
          <span
            className="min-w-0 max-w-[13rem] truncate"
            title={modelLabel}
            data-testid="invocation-table-model"
          >
            {renderInvocationModelChip(row.modelValue, {
              t,
              hasMismatch: row.modelHasMismatch,
              requestModel: row.modelHasMismatch ? row.requestModelValue : undefined,
              className: "max-w-full",
              textClassName: "font-mono text-[11px] text-base-content/68",
              iconClassName: "h-3.5 w-3.5",
              title: modelLabel,
            })}
          </span>
          {secondaryChips.length > 0 ? (
            <span className="flex min-w-0 flex-wrap items-center gap-1">{secondaryChips}</span>
          ) : null}
        </div>

        <div className="mt-1.5 flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1 border-t border-base-300/55 pt-1.5 text-[10px] text-base-content/65">
          <span className="min-w-0 font-mono tabular-nums">
            Hit {cacheHitRate} · {t("table.column.inputTokens")} {row.inputTokensValue} ·{" "}
            {t("table.column.cacheInputTokens")} {row.cacheInputTokensValue} ·{" "}
            {t("table.column.outputTokens")} {row.outputTokensValue}
          </span>
          <span className="min-w-0 font-mono tabular-nums text-base-content/82">
            {row.outputReasoningBreakdownValue} · {t("table.card.cacheWrite")}{" "}
            {row.cacheWriteTokensValue} · {t("table.card.requestCompression")}{" "}
            <span>{row.requestCompressionAlgorithmValue}</span> · {t("table.column.totalTokens")}{" "}
            {row.totalTokensValue} · {row.costValue}
          </span>
        </div>

        {row.collapsedErrorSummary ? (
          <div
            className="mt-3 min-w-0 border-t border-error/20 pt-2 text-xs text-error/90"
            title={row.errorMessage || row.collapsedErrorSummary}
          >
            <span className="font-semibold">{t("table.column.error")}:</span>{" "}
            {row.collapsedErrorSummary}
          </div>
        ) : null}

        {isExpanded ? (
          <div
            id={cardDetailId}
            className="mt-3 min-w-0 border-t border-base-300/70 pt-3"
            data-invocation-detail
          >
            <InvocationWorkflowDetailPanel
              record={row.record}
              focusedAttemptId={isHighlighted ? (scrollTarget?.attemptId ?? null) : null}
              size={isMdUp ? "default" : "compact"}
              onOpenUpstreamAccount={onOpenUpstreamAccount}
            />
          </div>
        ) : null}
      </article>
    );
  };

  return (
    <div
      className="space-y-3"
      ref={setContainerElement}
      data-testid="invocation-table-scroll"
      onPointerDownCapture={scheduleHighlightClear}
      onKeyDownCapture={scheduleHighlightClear}
    >
      <section
        className="border-b border-base-300/65 pb-3"
        aria-label={summaryTitle}
        data-testid="invocation-card-summary"
      >
        <div className="flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-2">
          <h3 className="text-sm font-semibold text-base-content">{summaryTitle}</h3>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-base-content/62">
            <span className="inline-flex items-center gap-1">
              <span className="h-1.5 w-1.5 rounded-full bg-success" aria-hidden />
              {locale === "zh" ? "成功" : "Success"} {listSummary.successful}
            </span>
            <span className="inline-flex items-center gap-1">
              <span className="h-1.5 w-1.5 rounded-full bg-error" aria-hidden />
              {locale === "zh" ? "失败" : "Failed"} {listSummary.failed}
            </span>
            <span className="inline-flex items-center gap-1">
              <span className="h-1.5 w-1.5 rounded-full bg-info" aria-hidden />
              {locale === "zh" ? "进行中" : "In flight"} {listSummary.running}
            </span>
          </div>
        </div>
        <div className="mt-2 grid grid-cols-2 gap-2 lg:grid-cols-4">
          {summaryMetrics.map((metric) => (
            <div
              key={metric.key}
              className="min-w-0 rounded-md border border-base-300/75 bg-base-200/35 px-2.5 py-2"
              data-testid={`invocation-card-summary-${metric.key}`}
            >
              <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-[0.06em] text-base-content/58">
                <AppIcon
                  name={metric.icon}
                  className={cn("h-3.5 w-3.5", metric.tone)}
                  aria-hidden
                />
                <span>{metric.label}</span>
              </div>
              <div
                className={cn(
                  "mt-1 truncate font-mono text-base font-semibold tabular-nums",
                  metric.tone,
                )}
              >
                {metric.value}
              </div>
              <div className="mt-0.5 truncate text-[10px] text-base-content/58">
                {metric.detail}
              </div>
            </div>
          ))}
        </div>
      </section>
      <div data-testid="invocation-card-list" data-invocation-card-list>
        {paddingTop > 0 ? <div aria-hidden="true" style={{ height: paddingTop }} /> : null}
        {fallbackVirtualRows.map((virtualRow) => {
          const row = rows[virtualRow.index];
          return row ? renderCard(row, virtualRow.index) : null;
        })}
        {paddingBottom > 0 ? <div aria-hidden="true" style={{ height: paddingBottom }} /> : null}
      </div>
    </div>
  );
}

export const InvocationTable = InvocationCardList;
