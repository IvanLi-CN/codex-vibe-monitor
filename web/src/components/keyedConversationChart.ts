import type {
  ConversationRequestOutcome,
  ConversationRequestPoint,
} from "../lib/api";
import { resolveConversationRequestPointOutcome } from "../lib/conversationRequestPoint";

export interface KeyedConversationChartRecord {
  last24hRequests: ConversationRequestPoint[];
}

interface ConversationChartSegment {
  startEpoch: number;
  endEpoch: number;
  cumulativeTokens: number;
  outcome: ConversationRequestOutcome;
  point: ConversationRequestPoint;
}

export const FALLBACK_CELL = "—";

function parseEpoch(raw?: string | null) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  if (Number.isNaN(epoch)) return null;
  return Math.floor(epoch / 1000);
}

export function resolveConversationRangeEpochs(
  rangeStart: string,
  rangeEnd: string,
) {
  const rangeStartEpoch = parseEpoch(rangeStart);
  const rangeEndEpoch = parseEpoch(rangeEnd);
  if (
    rangeStartEpoch == null ||
    rangeEndEpoch == null ||
    rangeEndEpoch <= rangeStartEpoch
  ) {
    return null;
  }
  return { rangeStartEpoch, rangeEndEpoch };
}

export function buildConversationSegments(
  points: ConversationRequestPoint[],
  rangeStartEpoch: number,
  rangeEndEpoch: number,
): ConversationChartSegment[] {
  if (points.length === 0 || rangeEndEpoch <= rangeStartEpoch) return [];
  const sorted = [...points].sort((a, b) => {
    const aEpoch = parseEpoch(a.occurredAt) ?? 0;
    const bEpoch = parseEpoch(b.occurredAt) ?? 0;
    return aEpoch - bEpoch;
  });

  const segments: ConversationChartSegment[] = [];
  for (let index = 0; index < sorted.length; index += 1) {
    const current = sorted[index];
    const next = sorted[index + 1];
    const currentEpoch = parseEpoch(current.occurredAt);
    if (currentEpoch == null) continue;
    const startEpoch = Math.max(
      rangeStartEpoch,
      Math.min(rangeEndEpoch, currentEpoch),
    );
    const nextEpoch = next
      ? (parseEpoch(next.occurredAt) ?? rangeEndEpoch)
      : rangeEndEpoch;
    const endEpoch = Math.max(startEpoch, Math.min(rangeEndEpoch, nextEpoch));
    if (endEpoch <= startEpoch) continue;

    segments.push({
      startEpoch,
      endEpoch,
      cumulativeTokens: Math.max(0, current.cumulativeTokens),
      outcome: resolveConversationRequestPointOutcome(current),
      point: current,
    });
  }

  return segments;
}

export function findVisibleConversationChartMax<
  TConversation extends KeyedConversationChartRecord,
>(conversations: TConversation[], rangeStart: string, rangeEnd: string) {
  const range = resolveConversationRangeEpochs(rangeStart, rangeEnd);
  if (!range) return 0;
  return Math.max(
    ...conversations.flatMap((conversation) =>
      buildConversationSegments(
        conversation.last24hRequests,
        range.rangeStartEpoch,
        range.rangeEndEpoch,
      ).map((segment) => segment.cumulativeTokens),
    ),
    0,
  );
}
