import type { ApiInvocation } from "./api";
import { isFiniteNonNegativeMilliseconds, isFinitePositiveMilliseconds } from "./invocationTiming";

export type InvocationCompactTimingState =
  | "requesting"
  | "awaitingToken"
  | "responding"
  | "terminal";

export interface InvocationCompactTimingInput {
  record: Pick<
    ApiInvocation,
    | "tReqReadMs"
    | "tReqParseMs"
    | "tUpstreamConnectMs"
    | "tUpstreamTtfbMs"
    | "firstTokenMs"
    | "tUpstreamStreamMs"
  >;
  occurredAtEpoch: number | null;
  isInFlight: boolean;
  nowMs: number;
}

export interface InvocationCompactTimingDisplay {
  state: InvocationCompactTimingState;
  requestMs: number | null;
  requestProvisional: boolean;
  ttftMs: number | null;
  ttftProvisional: boolean;
  responseMs: number | null;
  responseProvisional: boolean;
}

export interface InvocationCompactTimingPresentation {
  requestMs: number | null;
  requestProvisional: boolean;
  ttftMs: number | null;
  ttftProvisional: boolean;
  responseMs: number | null;
  responseProvisional: boolean;
}

function localElapsedMs(occurredAtEpoch: number | null, nowMs: number) {
  if (occurredAtEpoch == null || !Number.isFinite(nowMs)) return null;
  return Math.max(0, nowMs - occurredAtEpoch);
}

export function resolveFirstResponseByteTotalMs(
  record: Pick<
    ApiInvocation,
    "tReqReadMs" | "tReqParseMs" | "tUpstreamConnectMs" | "tUpstreamTtfbMs"
  >,
) {
  if (
    !isFiniteNonNegativeMilliseconds(record.tReqReadMs) ||
    !isFiniteNonNegativeMilliseconds(record.tReqParseMs) ||
    !isFiniteNonNegativeMilliseconds(record.tUpstreamConnectMs) ||
    !isFinitePositiveMilliseconds(record.tUpstreamTtfbMs)
  ) {
    return null;
  }
  return (
    record.tReqReadMs + record.tReqParseMs + record.tUpstreamConnectMs + record.tUpstreamTtfbMs
  );
}

export function buildInvocationCompactTiming(
  input: InvocationCompactTimingInput,
): InvocationCompactTimingDisplay {
  const requestMs = resolveFirstResponseByteTotalMs(input.record);
  const measuredTtftMs = isFiniteNonNegativeMilliseconds(input.record.firstTokenMs)
    ? input.record.firstTokenMs
    : null;
  const measuredResponseMs = isFinitePositiveMilliseconds(input.record.tUpstreamStreamMs)
    ? input.record.tUpstreamStreamMs
    : null;

  if (!input.isInFlight) {
    return {
      state: "terminal",
      requestMs,
      requestProvisional: false,
      ttftMs: measuredTtftMs,
      ttftProvisional: false,
      responseMs: requestMs != null ? measuredResponseMs : null,
      responseProvisional: false,
    };
  }

  const elapsedMs = localElapsedMs(input.occurredAtEpoch, input.nowMs);
  if (measuredTtftMs != null) {
    const responseMs =
      measuredResponseMs ??
      (requestMs != null && elapsedMs != null ? Math.max(0, elapsedMs - requestMs) : null);
    return {
      state: "responding",
      requestMs,
      requestProvisional: false,
      ttftMs: measuredTtftMs,
      ttftProvisional: false,
      responseMs,
      responseProvisional: measuredResponseMs == null && responseMs != null,
    };
  }

  if (requestMs != null) {
    return {
      state: "awaitingToken",
      requestMs,
      requestProvisional: false,
      ttftMs: elapsedMs,
      ttftProvisional: elapsedMs != null,
      responseMs: null,
      responseProvisional: false,
    };
  }

  return {
    state: "requesting",
    requestMs: elapsedMs,
    requestProvisional: elapsedMs != null,
    ttftMs: null,
    ttftProvisional: false,
    responseMs: null,
    responseProvisional: false,
  };
}

function reconcileValue(
  value: number | null,
  provisional: boolean,
  previousValue: number | null,
  previousProvisional: boolean,
  terminal: boolean,
) {
  if (terminal || value == null) {
    return { value, provisional };
  }
  if (provisional) {
    return {
      value: previousProvisional && previousValue != null ? Math.max(previousValue, value) : value,
      provisional,
    };
  }
  if (previousProvisional && previousValue != null && value < previousValue) {
    return { value: previousValue, provisional: true };
  }
  return { value, provisional };
}

export function reconcileInvocationCompactTiming(
  timing: InvocationCompactTimingDisplay,
  previous: InvocationCompactTimingPresentation | null,
): InvocationCompactTimingPresentation {
  const terminal = timing.state === "terminal";
  const request = reconcileValue(
    timing.requestMs,
    timing.requestProvisional,
    previous?.requestMs ?? null,
    previous?.requestProvisional ?? false,
    terminal,
  );
  const ttft = reconcileValue(
    timing.ttftMs,
    timing.ttftProvisional,
    previous?.ttftMs ?? null,
    previous?.ttftProvisional ?? false,
    terminal,
  );
  const response = reconcileValue(
    timing.responseMs,
    timing.responseProvisional,
    previous?.responseMs ?? null,
    previous?.responseProvisional ?? false,
    terminal,
  );
  return {
    requestMs: request.value,
    requestProvisional: request.provisional,
    ttftMs: ttft.value,
    ttftProvisional: ttft.provisional,
    responseMs: response.value,
    responseProvisional: response.provisional,
  };
}

export function formatInvocationCompactTimingValue(
  value: number | null | undefined,
  localeTag: string,
  accepts: (value: number | null | undefined) => value is number = isFiniteNonNegativeMilliseconds,
) {
  if (!accepts(value)) return "--";

  const seconds = value / 1000;
  const roundedTenths = Math.round(seconds * 10) / 10;
  const fractionDigits = roundedTenths >= 100 ? 0 : 1;
  const rounded = Number(seconds.toFixed(fractionDigits));

  return `${rounded.toLocaleString(localeTag, {
    useGrouping: false,
    minimumFractionDigits: 0,
    maximumFractionDigits: fractionDigits,
  })}s`;
}
