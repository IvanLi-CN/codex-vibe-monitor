import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { AnimatedDigits } from "./AnimatedDigits";
import {
  type AdaptiveCurrencyProfile,
  type AdaptiveDisplayValueSpec,
  type AdaptiveMetricValueKind,
  buildAdaptiveMetricSpec,
} from "./adaptiveMetricValueSpec";

const ADAPTIVE_METRIC_COMPACT_GUTTER_PX = 12;
const ADAPTIVE_METRIC_UPGRADE_HEADROOM_PX = 6;

interface AdaptiveDisplayValueProps {
  spec: AdaptiveDisplayValueSpec;
  className?: string;
  title?: string;
  animateDigits?: boolean;
  "data-testid"?: string;
}

interface AdaptiveMetricValueProps {
  value: number;
  localeTag: string;
  kind?: AdaptiveMetricValueKind;
  currencyProfile?: AdaptiveCurrencyProfile;
  className?: string;
  "data-testid"?: string;
}

function useAdaptiveCandidateSelection(spec: AdaptiveDisplayValueSpec) {
  const containerRef = useRef<HTMLSpanElement | null>(null);
  const measureRefs = useRef<Array<HTMLSpanElement | null>>([]);
  const [selectedCandidateKey, setSelectedCandidateKey] = useState<string>(
    () => spec.candidates[0]?.key ?? "full",
  );

  const evaluateOverflow = useCallback(() => {
    const container = containerRef.current;
    if (!container) return;

    const availableWidth = container.clientWidth;
    if (availableWidth <= 0) return;

    const measures = measureRefs.current;
    const candidateWidths = spec.candidates.map((candidate, index) => ({
      candidate,
      index,
      requiredWidth: measures[index]?.scrollWidth ?? 0,
    }));
    const currentIndex = Math.max(
      0,
      spec.candidates.findIndex((candidate) => candidate.key === selectedCandidateKey),
    );
    const currentCandidateWidth = candidateWidths[currentIndex]?.requiredWidth ?? 0;

    const fitsWithinWidth = (requiredWidth: number, extraHeadroomPx: number) =>
      requiredWidth > 0 &&
      requiredWidth + ADAPTIVE_METRIC_COMPACT_GUTTER_PX + extraHeadroomPx <= availableWidth;

    let nextCandidate = spec.candidates[currentIndex] ?? spec.candidates.at(-1);

    if (!fitsWithinWidth(currentCandidateWidth, 0)) {
      nextCandidate = spec.candidates.at(-1) ?? nextCandidate;
      for (let index = currentIndex + 1; index < candidateWidths.length; index += 1) {
        if (fitsWithinWidth(candidateWidths[index]?.requiredWidth ?? 0, 0)) {
          nextCandidate = candidateWidths[index]?.candidate ?? nextCandidate;
          break;
        }
      }
    } else {
      for (let index = 0; index < currentIndex; index += 1) {
        if (
          fitsWithinWidth(
            candidateWidths[index]?.requiredWidth ?? 0,
            ADAPTIVE_METRIC_UPGRADE_HEADROOM_PX,
          )
        ) {
          nextCandidate = candidateWidths[index]?.candidate ?? nextCandidate;
          break;
        }
      }
    }

    setSelectedCandidateKey((current) =>
      current === (nextCandidate?.key ?? current) ? current : (nextCandidate?.key ?? current),
    );
  }, [selectedCandidateKey, spec.candidates]);

  useLayoutEffect(() => {
    evaluateOverflow();
    const frame = window.requestAnimationFrame(() => {
      evaluateOverflow();
    });

    return () => {
      window.cancelAnimationFrame(frame);
    };
  }, [evaluateOverflow]);

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return undefined;

    window.addEventListener("resize", evaluateOverflow);

    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.removeEventListener("resize", evaluateOverflow);
      };
    }

    const observer = new ResizeObserver(() => {
      evaluateOverflow();
    });
    observer.observe(container);
    for (const measure of measureRefs.current) {
      if (measure) observer.observe(measure);
    }

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", evaluateOverflow);
    };
  }, [evaluateOverflow]);

  const selectedCandidate =
    spec.candidates.find((candidate) => candidate.key === selectedCandidateKey) ??
    spec.candidates[0];

  return {
    containerRef,
    measureRefs,
    selectedCandidate,
  };
}

export function AdaptiveDisplayValue({
  spec,
  className,
  title,
  animateDigits = false,
  "data-testid": dataTestId,
}: AdaptiveDisplayValueProps) {
  const { containerRef, measureRefs, selectedCandidate } = useAdaptiveCandidateSelection(spec);
  const shouldAnimateDigits =
    animateDigits &&
    selectedCandidate?.key === spec.candidates[0]?.key &&
    !selectedCandidate?.compact;
  const resolvedTitle =
    title ?? (selectedCandidate?.value !== spec.fullValue ? spec.fullValue : undefined);

  return (
    <span
      ref={containerRef}
      data-adaptive-metric-container="true"
      className={`relative block min-w-0 max-w-full overflow-hidden whitespace-nowrap tabular-nums ${className ?? ""}`}
    >
      {spec.candidates.map((candidate, index) => (
        <span
          key={`${candidate.key}-${candidate.value}`}
          ref={(node) => {
            measureRefs.current[index] = node;
          }}
          aria-hidden
          data-adaptive-metric-measure="true"
          data-adaptive-metric-measure-kind={candidate.compact ? "compact" : "full"}
          data-adaptive-metric-measure-index={String(index)}
          className="pointer-events-none invisible absolute left-0 top-0 whitespace-nowrap tabular-nums"
        >
          {candidate.value}
        </span>
      ))}
      <span
        data-adaptive-metric-visible="true"
        data-compact={selectedCandidate?.compact ? "true" : "false"}
        data-compact-precision={selectedCandidate?.precisionLabel ?? "full"}
        data-candidate-key={selectedCandidate?.key ?? "full"}
        data-testid={dataTestId}
        title={resolvedTitle}
        className={cn("block max-w-full overflow-hidden whitespace-nowrap", className)}
      >
        {shouldAnimateDigits ? (
          <AnimatedDigits value={selectedCandidate?.value ?? spec.fullValue} />
        ) : (
          (selectedCandidate?.value ?? spec.fullValue)
        )}
      </span>
    </span>
  );
}

export function AdaptiveMetricValue({
  value,
  localeTag,
  kind = "number",
  currencyProfile,
  className,
  "data-testid": dataTestId,
}: AdaptiveMetricValueProps) {
  const spec = useMemo(
    () => buildAdaptiveMetricSpec(value, localeTag, kind, { currencyProfile }),
    [currencyProfile, kind, localeTag, value],
  );

  return (
    <AdaptiveDisplayValue
      spec={spec}
      className={className}
      data-testid={dataTestId}
      animateDigits={kind === "number" || kind === "integer"}
    />
  );
}

export type {
  AdaptiveCurrencyProfile,
  AdaptiveDisplayValueSpec,
  AdaptiveMetricValueKind,
} from "./adaptiveMetricValueSpec";
