import type { PointerEvent as ReactPointerEvent, RefObject } from "react";
import { useCallback, useEffect, useMemo, useRef } from "react";

const TRANSITION_PADDING_PX = 8;
const STATIONARY_CLOSE_DELAY_MS = 500;

type Point = { x: number; y: number };
type Rect = Pick<DOMRect, "left" | "right" | "top" | "bottom">;
type TransitionFrom = "trigger" | "content";

function cross(origin: Point, first: Point, second: Point) {
  return (
    (first.x - origin.x) * (second.y - origin.y) - (first.y - origin.y) * (second.x - origin.x)
  );
}

function buildTransitionArea(exitPoint: Point, target: Rect): Point[] {
  const padding = TRANSITION_PADDING_PX;
  const points = [
    { x: exitPoint.x - padding, y: exitPoint.y - padding },
    { x: exitPoint.x - padding, y: exitPoint.y + padding },
    { x: exitPoint.x + padding, y: exitPoint.y - padding },
    { x: exitPoint.x + padding, y: exitPoint.y + padding },
    { x: target.left - padding, y: target.top - padding },
    { x: target.right + padding, y: target.top - padding },
    { x: target.right + padding, y: target.bottom + padding },
    { x: target.left - padding, y: target.bottom + padding },
  ].sort((first, second) => first.x - second.x || first.y - second.y);

  const lower: Point[] = [];
  for (const point of points) {
    while (
      lower.length >= 2 &&
      cross(lower[lower.length - 2], lower[lower.length - 1], point) <= 0
    ) {
      lower.pop();
    }
    lower.push(point);
  }

  const upper: Point[] = [];
  for (const point of [...points].reverse()) {
    while (
      upper.length >= 2 &&
      cross(upper[upper.length - 2], upper[upper.length - 1], point) <= 0
    ) {
      upper.pop();
    }
    upper.push(point);
  }

  lower.pop();
  upper.pop();
  return [...lower, ...upper];
}

function isPointInArea(point: Point, area: Point[]) {
  let inside = false;
  for (let index = 0, previous = area.length - 1; index < area.length; previous = index++) {
    const currentPoint = area[index];
    const previousPoint = area[previous];
    const intersects =
      currentPoint.y > point.y !== previousPoint.y > point.y &&
      point.x <
        ((previousPoint.x - currentPoint.x) * (point.y - currentPoint.y)) /
          (previousPoint.y - currentPoint.y) +
          currentPoint.x;
    if (intersects) inside = !inside;
  }
  return inside;
}

export function usePointerTransitionGuard({
  triggerRef,
  contentRef,
  onClose,
  pinned = false,
  enabled = true,
}: {
  triggerRef: RefObject<HTMLElement | null>;
  contentRef: RefObject<HTMLElement | null>;
  onClose: () => void;
  pinned?: boolean;
  enabled?: boolean;
}) {
  const onCloseRef = useRef(onClose);
  const pinnedRef = useRef(pinned);
  const enabledRef = useRef(enabled);
  const activeRef = useRef(false);
  const closeTimerRef = useRef<number | null>(null);
  const transitionAreaRef = useRef<Point[]>([]);
  const pointerMoveHandlerRef = useRef<(event: PointerEvent) => void>(() => undefined);

  onCloseRef.current = onClose;
  pinnedRef.current = pinned;
  enabledRef.current = enabled;

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current === null) return;
    window.clearTimeout(closeTimerRef.current);
    closeTimerRef.current = null;
  }, []);

  const cancel = useCallback(() => {
    activeRef.current = false;
    transitionAreaRef.current = [];
    clearCloseTimer();
    document.removeEventListener("pointermove", pointerMoveHandlerRef.current, true);
  }, [clearCloseTimer]);

  const close = useCallback(() => {
    cancel();
    if (enabledRef.current && !pinnedRef.current) onCloseRef.current();
  }, [cancel]);

  const scheduleStationaryClose = useCallback(() => {
    clearCloseTimer();
    closeTimerRef.current = window.setTimeout(close, STATIONARY_CLOSE_DELAY_MS);
  }, [clearCloseTimer, close]);

  const handlePointerMove = useCallback(
    (event: PointerEvent) => {
      if (!activeRef.current || event.pointerType === "touch") return;
      const target = event.target;
      if (
        target instanceof Node &&
        (triggerRef.current?.contains(target) || contentRef.current?.contains(target))
      ) {
        cancel();
        return;
      }
      if (!isPointInArea({ x: event.clientX, y: event.clientY }, transitionAreaRef.current)) {
        close();
        return;
      }
      scheduleStationaryClose();
    },
    [cancel, close, contentRef, scheduleStationaryClose, triggerRef],
  );
  pointerMoveHandlerRef.current = handlePointerMove;

  const start = useCallback(
    (from: TransitionFrom, event: ReactPointerEvent<HTMLElement>) => {
      if (!enabledRef.current || pinnedRef.current || event.pointerType === "touch") return;
      cancel();
      const targetElement = from === "trigger" ? contentRef.current : triggerRef.current;
      const rect = targetElement?.getBoundingClientRect();
      if (!rect) {
        close();
        return;
      }
      transitionAreaRef.current = buildTransitionArea({ x: event.clientX, y: event.clientY }, rect);
      activeRef.current = true;
      document.addEventListener("pointermove", pointerMoveHandlerRef.current, true);
      scheduleStationaryClose();
    },
    [cancel, close, contentRef, scheduleStationaryClose, triggerRef],
  );

  useEffect(() => {
    if (!enabled || pinned) cancel();
  }, [cancel, enabled, pinned]);

  useEffect(() => cancel, [cancel]);

  return useMemo(() => ({ cancel, start }), [cancel, start]);
}
