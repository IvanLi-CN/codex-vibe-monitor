import { useCallback, useEffect, useRef, useState } from "react";
import type { TaskTimelineCoverage, TaskTimelinePage, TaskTimelineSegment } from "../lib/api";
import { fetchManagedTaskTimeline } from "../lib/api";
import { useSubscriptionTopic } from "./useSubscriptionTopic";

const TIMELINE_PAGE_SIZE = 500;
const TIMELINE_WINDOW_HOURS = 12;

interface TaskTimelineRevision {
  watermark: number;
  observedAt: string;
}

interface TimelineSnapshot {
  watermark: number;
  segments: TaskTimelineSegment[];
  coverage: TaskTimelineCoverage[];
  from: string;
  to: string;
}

interface TimelinePageBatch {
  watermark: number;
  segments: TaskTimelineSegment[];
  coverage: TaskTimelineCoverage[];
  from: string;
  to: string;
  resetRequired: boolean;
}

async function readTimelinePages(
  afterRevision?: number,
  window?: Pick<TimelineSnapshot, "from" | "to">,
): Promise<TimelinePageBatch> {
  const segments: TaskTimelineSegment[] = [];
  let coverage: TaskTimelineCoverage[] = [];
  let page: TaskTimelinePage = await fetchManagedTaskTimeline({
    ...(window ? { from: window.from, to: window.to } : { windowHours: TIMELINE_WINDOW_HOURS }),
    afterRevision,
    limit: TIMELINE_PAGE_SIZE,
  });
  const bounds = window ?? { from: page.windowStart, to: page.windowEnd };
  if (page.resetRequired) {
    return { ...bounds, watermark: page.watermark, segments, coverage, resetRequired: true };
  }
  const watermark = page.watermark;
  segments.push(...page.segments);
  coverage = page.coverage;
  while (page.nextCursor) {
    page = await fetchManagedTaskTimeline({ cursor: page.nextCursor, limit: TIMELINE_PAGE_SIZE });
    if (page.resetRequired) {
      return {
        ...bounds,
        watermark: page.watermark,
        segments: [],
        coverage: [],
        resetRequired: true,
      };
    }
    if (page.watermark !== watermark) {
      throw new Error("Timeline page watermark changed during pagination");
    }
    segments.push(...page.segments);
    coverage = page.coverage;
  }
  return { ...bounds, watermark, segments, coverage, resetRequired: false };
}

function mergeTimelineSegments(
  current: TaskTimelineSegment[],
  updates: TaskTimelineSegment[],
  from: string,
  to: string,
) {
  const byId = new Map(current.map((segment) => [segment.segmentId, segment]));
  for (const segment of updates) {
    const previous = byId.get(segment.segmentId);
    if (!previous || segment.revision > previous.revision) {
      byId.set(segment.segmentId, segment);
    }
  }
  const startBound = Date.parse(from);
  const endBound = Date.parse(to);
  return [...byId.values()]
    .filter((segment) => {
      const start = Date.parse(segment.startedAt);
      const end = Date.parse(segment.finishedAt ?? segment.lastObservedAt);
      return start <= endBound && end >= startBound;
    })
    .sort((left, right) => left.startedAt.localeCompare(right.startedAt));
}

export function useManagedTaskTimeline() {
  const topic = useSubscriptionTopic<TaskTimelineRevision>({
    topic: "system.managed-tasks.timeline",
  });
  const [snapshot, setSnapshot] = useState<TimelineSnapshot | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);
  const committed = useRef<TimelineSnapshot | null>(null);
  const appliedWatermark = useRef<number | null>(null);
  const desiredWatermark = useRef<number | null>(null);
  const retryRequested = useRef(false);
  const syncRunning = useRef(false);
  const mounted = useRef(true);
  const stale = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (topic.error) stale.current = true;
  }, [topic.error]);

  const synchronize = useCallback(async () => {
    if (syncRunning.current) return;
    syncRunning.current = true;
    let cursorResets = 0;
    let failed = false;
    try {
      while (mounted.current) {
        if (appliedWatermark.current == null) {
          const batch = await readTimelinePages();
          if (batch.resetRequired) {
            cursorResets += 1;
            if (cursorResets > 3) throw new Error("Timeline snapshot cursor repeatedly expired");
            continue;
          }
          const next: TimelineSnapshot = {
            watermark: batch.watermark,
            segments: batch.segments,
            coverage: batch.coverage,
            from: batch.from,
            to: batch.to,
          };
          if (!mounted.current) return;
          committed.current = next;
          appliedWatermark.current = batch.watermark;
          setSnapshot(next);
          setRequestError(null);
          stale.current = false;
          cursorResets = 0;
          continue;
        }

        const target = desiredWatermark.current ?? appliedWatermark.current;
        if (target <= appliedWatermark.current && !retryRequested.current) break;

        const batch = await readTimelinePages(
          appliedWatermark.current,
          committed.current ?? undefined,
        );
        if (batch.resetRequired) {
          cursorResets += 1;
          if (cursorResets > 3) {
            throw new Error("Timeline cursor repeatedly expired; baseline required");
          }
          appliedWatermark.current = null;
          continue;
        }
        const previous = committed.current;
        const next: TimelineSnapshot = {
          watermark: batch.watermark,
          segments: mergeTimelineSegments(
            previous?.segments ?? [],
            batch.segments,
            batch.from,
            batch.to,
          ),
          coverage: batch.coverage,
          from: batch.from,
          to: batch.to,
        };
        if (!mounted.current) return;
        committed.current = next;
        appliedWatermark.current = batch.watermark;
        retryRequested.current = false;
        setSnapshot(next);
        setRequestError(null);
        stale.current = false;
        cursorResets = 0;
      }
    } catch (reason: unknown) {
      failed = true;
      if (mounted.current) {
        const message = reason instanceof Error ? reason.message : String(reason);
        setRequestError(message);
        stale.current = true;
        retryRequested.current = false;
      }
    } finally {
      syncRunning.current = false;
      if (
        mounted.current &&
        !failed &&
        desiredWatermark.current != null &&
        appliedWatermark.current != null &&
        (desiredWatermark.current > appliedWatermark.current || retryRequested.current)
      ) {
        queueMicrotask(() => void synchronize());
      }
    }
  }, []);

  useEffect(() => {
    if (topic.data) {
      desiredWatermark.current = Math.max(
        desiredWatermark.current ?? topic.data.watermark,
        topic.data.watermark,
      );
      if (stale.current) retryRequested.current = true;
    }
    void synchronize();
  }, [topic.data, synchronize]);

  return {
    segments: snapshot?.segments ?? [],
    coverage: snapshot?.coverage ?? [],
    watermark: snapshot?.watermark ?? null,
    error: topic.error ?? requestError,
    isLoading: snapshot == null && requestError == null,
    lastReceivedAt: topic.lastReceivedAt,
    refresh: topic.refresh,
  };
}
