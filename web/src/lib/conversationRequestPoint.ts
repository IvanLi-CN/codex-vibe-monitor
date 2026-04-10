import type {
  ApiInvocation,
  ConversationRequestOutcome,
  ConversationRequestPoint,
} from "./api";

function normalizeOutcome(
  value: ConversationRequestOutcome | null | undefined,
): ConversationRequestOutcome | null {
  return value === "success" ||
    value === "failure" ||
    value === "neutral" ||
    value === "in_flight"
    ? value
    : null;
}

function normalizeStatus(value: string | null | undefined) {
  return value?.trim().toLowerCase() ?? "";
}

function normalizeFailureClass(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase() ?? "";
  return normalized.length > 0 ? normalized : null;
}

export function resolveConversationRequestPointOutcome(
  point: Pick<ConversationRequestPoint, "status" | "isSuccess" | "outcome">,
): ConversationRequestOutcome {
  const explicit = normalizeOutcome(point.outcome);
  if (explicit) return explicit;

  const status = normalizeStatus(point.status);
  if (status === "running" || status === "pending") {
    return "in_flight";
  }
  if (status === "" || status === "unknown") {
    return "neutral";
  }
  return point.isSuccess ? "success" : "failure";
}

export function resolvePromptCacheInvocationOutcome(
  record: Pick<ApiInvocation, "status" | "failureClass" | "errorMessage">,
): ConversationRequestOutcome {
  const status = normalizeStatus(record.status);
  if (status === "running" || status === "pending") {
    return "in_flight";
  }

  const failureClass = normalizeFailureClass(record.failureClass);
  if (failureClass != null && failureClass !== "none") {
    return "failure";
  }

  const hasErrorMessage = (record.errorMessage?.trim().length ?? 0) > 0;
  if (
    status === "success" ||
    status === "completed" ||
    (status === "http_200" && !hasErrorMessage)
  ) {
    return "success";
  }

  if (status === "" || status === "unknown") {
    return "neutral";
  }

  if (!hasErrorMessage && failureClass === "none") {
    return "neutral";
  }

  return "failure";
}
