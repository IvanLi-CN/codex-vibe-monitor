import type { TranslationKey } from "../i18n";
import type { ApiInvocation, InvocationLivePhase, InvocationPhaseCounts } from "./api";
import { resolveInvocationDisplayStatus } from "./invocationStatus";

type InvocationPhaseSource = Pick<
  ApiInvocation,
  | "status"
  | "failureClass"
  | "livePhase"
  | "upstreamAccountId"
  | "tReqReadMs"
  | "tReqParseMs"
  | "tUpstreamConnectMs"
  | "tUpstreamTtfbMs"
  | "firstTokenMs"
>;

export type InvocationPhaseChipTone = "warning" | "info" | "secondary";

export interface InvocationPhaseDisplay {
  phase: InvocationLivePhase;
  labelKey: TranslationKey;
  chipTone: InvocationPhaseChipTone;
}

export const EMPTY_INVOCATION_PHASE_COUNTS: InvocationPhaseCounts = {
  queued: 0,
  requesting: 0,
  responding: 0,
};

function normalizePhase(value: string | null | undefined): InvocationLivePhase | null {
  if (value === "queued" || value === "requesting" || value === "responding") {
    return value;
  }
  return null;
}

function hasFiniteTiming(value: number | null | undefined): boolean {
  return typeof value === "number" && Number.isFinite(value) && value > 0;
}

function resolvePreResponsePhase(record: InvocationPhaseSource): InvocationLivePhase {
  if (
    record.upstreamAccountId != null ||
    hasFiniteTiming(record.tUpstreamConnectMs) ||
    hasFiniteTiming(record.tReqReadMs) ||
    hasFiniteTiming(record.tReqParseMs)
  ) {
    return "requesting";
  }
  return "queued";
}

export function resolveInvocationLivePhase(
  record: InvocationPhaseSource,
): InvocationLivePhase | null {
  const displayStatus = resolveInvocationDisplayStatus(record);
  const normalizedStatus = displayStatus.trim().toLowerCase();
  if (normalizedStatus !== "running" && normalizedStatus !== "pending") {
    return null;
  }

  const hasFirstToken = hasFiniteTiming(record.firstTokenMs);
  if (normalizedStatus === "pending") return "queued";
  if (hasFirstToken) return "responding";
  const explicitPhase = normalizePhase(record.livePhase);
  if (explicitPhase === "responding" && !hasFirstToken) {
    return resolvePreResponsePhase(record);
  }
  if (explicitPhase) return explicitPhase;
  return resolvePreResponsePhase(record);
}

export function getInvocationPhaseDisplay(phase: InvocationLivePhase): InvocationPhaseDisplay {
  if (phase === "responding") {
    return {
      phase,
      labelKey: "table.status.responding",
      chipTone: "secondary",
    };
  }
  if (phase === "requesting") {
    return {
      phase,
      labelKey: "table.status.requesting",
      chipTone: "info",
    };
  }
  return {
    phase,
    labelKey: "table.status.queued",
    chipTone: "warning",
  };
}

export function normalizeInvocationPhaseCounts(
  counts: InvocationPhaseCounts | null | undefined,
): InvocationPhaseCounts {
  return {
    queued: Math.max(0, counts?.queued ?? 0),
    requesting: Math.max(0, counts?.requesting ?? 0),
    responding: Math.max(0, counts?.responding ?? 0),
  };
}

export function sumInvocationPhaseCounts(counts: InvocationPhaseCounts | null | undefined): number {
  const normalized = normalizeInvocationPhaseCounts(counts);
  return normalized.queued + normalized.requesting + normalized.responding;
}
