import {
  type Dispatch,
  type MutableRefObject,
  type PointerEvent as ReactPointerEvent,
  type SetStateAction,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS,
  CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO,
  CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX,
  CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO,
  CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY,
  CONVERSATION_ACTIVITY_WHEEL_THRESHOLD,
  CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY,
  type ConversationActivityBucket,
  type ConversationActivityDragAxis,
  type ConversationActivityViewport,
  clampConversationActivityValue,
  isSameConversationActivityViewport,
  normalizeConversationActivityViewport,
  shiftConversationActivityViewport,
  zoomConversationActivityViewport,
} from "./PromptCacheActivityData";

type ConversationActivityDragState = {
  pointerId: number;
  startClientX: number;
  startClientY: number;
  currentClientX: number;
  currentClientY: number;
  axis: ConversationActivityDragAxis;
  viewport: ConversationActivityViewport;
};

export function useConversationActivityViewport(buckets: ConversationActivityBucket[]) {
  const identity = `${buckets.length}:${buckets[0]?.tooltipLabel ?? "empty"}:${buckets.at(-1)?.tooltipLabel ?? "empty"}`;
  const [viewport, setViewport] = useState<ConversationActivityViewport>({
    startIndex: 0,
    endIndex: Math.max(0, buckets.length - 1),
  });
  const viewportRef = useRef(viewport);
  useEffect(() => {
    viewportRef.current = viewport;
  }, [viewport]);
  const identityRef = useRef(identity);

  useEffect(() => {
    setViewport((current) => {
      if (identityRef.current !== identity) {
        identityRef.current = identity;
        return normalizeConversationActivityViewport(
          { startIndex: 0, endIndex: Math.max(0, buckets.length - 1) },
          buckets.length,
        );
      }
      return normalizeConversationActivityViewport(current, buckets.length);
    });
  }, [buckets.length, identity]);

  return { viewport, viewportRef, setViewport };
}

function useConversationActivityWheelSchedulers(
  bucketCount: number,
  setViewport: Dispatch<SetStateAction<ConversationActivityViewport>>,
) {
  const panDeltaRef = useRef(0);
  const panFrameRef = useRef<number | null>(null);
  const zoomDeltaRef = useRef(0);
  const zoomAnchorRef = useRef(0.5);
  const zoomFrameRef = useRef<number | null>(null);

  const schedulePan = useCallback(
    (deltaIndexes: number) => {
      panDeltaRef.current += deltaIndexes;
      if (panFrameRef.current != null) return;
      panFrameRef.current = window.requestAnimationFrame(() => {
        panFrameRef.current = null;
        const pending = panDeltaRef.current;
        panDeltaRef.current = 0;
        if (pending === 0) return;
        const delta =
          Math.round(pending) ||
          Math.sign(pending) *
            Math.max(
              1,
              Math.round(
                CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY *
                  CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS,
              ),
            );
        setViewport((current) => {
          const normalized = normalizeConversationActivityViewport(current, bucketCount);
          const next = shiftConversationActivityViewport(normalized, bucketCount, delta);
          return isSameConversationActivityViewport(normalized, next) ? current : next;
        });
      });
    },
    [bucketCount, setViewport],
  );

  const scheduleZoom = useCallback(
    (deltaY: number, anchorRatio: number) => {
      zoomDeltaRef.current += deltaY;
      zoomAnchorRef.current = anchorRatio;
      if (zoomFrameRef.current != null) return;
      zoomFrameRef.current = window.requestAnimationFrame(() => {
        zoomFrameRef.current = null;
        const pending = zoomDeltaRef.current;
        zoomDeltaRef.current = 0;
        if (pending === 0) return;
        setViewport((current) => {
          const normalized = normalizeConversationActivityViewport(current, bucketCount);
          const next = zoomConversationActivityViewport(
            normalized,
            bucketCount,
            pending * CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY,
            zoomAnchorRef.current,
          );
          return isSameConversationActivityViewport(normalized, next) ? current : next;
        });
      });
    },
    [bucketCount, setViewport],
  );

  useEffect(
    () => () => {
      if (panFrameRef.current != null) window.cancelAnimationFrame(panFrameRef.current);
      if (zoomFrameRef.current != null) window.cancelAnimationFrame(zoomFrameRef.current);
    },
    [],
  );

  return { schedulePan, scheduleZoom };
}

export function useConversationActivityWheel(
  buckets: ConversationActivityBucket[],
  viewportRef: MutableRefObject<ConversationActivityViewport>,
  interactionRef: MutableRefObject<HTMLDivElement | null>,
  setViewport: Dispatch<SetStateAction<ConversationActivityViewport>>,
) {
  const { schedulePan, scheduleZoom } = useConversationActivityWheelSchedulers(
    buckets.length,
    setViewport,
  );
  const getAnchorRatio = useCallback(
    (clientX: number) => {
      const rect = interactionRef.current?.getBoundingClientRect();
      return rect?.width
        ? clampConversationActivityValue((clientX - rect.left) / rect.width, 0, 1)
        : 0.5;
    },
    [interactionRef],
  );

  return useCallback(
    (event: WheelEvent) => {
      if (buckets.length <= CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS) return;
      const horizontal =
        Math.abs(event.deltaX) >= CONVERSATION_ACTIVITY_WHEEL_THRESHOLD &&
        Math.abs(event.deltaX) >= Math.abs(event.deltaY) &&
        !event.ctrlKey;
      const zoom = event.ctrlKey || event.metaKey || event.altKey;
      if (!horizontal && !zoom) return;
      event.preventDefault();
      if (horizontal) {
        const viewport = normalizeConversationActivityViewport(viewportRef.current, buckets.length);
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        schedulePan(
          (event.deltaX / Math.max(1, width)) * (viewport.endIndex - viewport.startIndex + 1),
        );
      } else {
        scheduleZoom(event.deltaY, getAnchorRatio(event.clientX));
      }
    },
    [buckets.length, getAnchorRatio, interactionRef, schedulePan, scheduleZoom, viewportRef],
  );
}

function useConversationActivityDragPreview(
  dragRef: MutableRefObject<ConversationActivityDragState | null>,
  dragPreviewLayerRef: MutableRefObject<HTMLDivElement | null>,
) {
  const previewOffsetRef = useRef(0);
  const previewFrameRef = useRef<number | null>(null);
  const resetPreview = useCallback(() => {
    previewOffsetRef.current = 0;
    if (dragPreviewLayerRef.current) dragPreviewLayerRef.current.style.transform = "";
  }, [dragPreviewLayerRef]);
  const schedulePreview = useCallback(() => {
    if (previewFrameRef.current != null) return;
    previewFrameRef.current = window.requestAnimationFrame(() => {
      previewFrameRef.current = null;
      const drag = dragRef.current;
      if (!drag) return;
      const offset = drag.currentClientX - drag.startClientX;
      if (offset === previewOffsetRef.current) return;
      previewOffsetRef.current = offset;
      if (dragPreviewLayerRef.current)
        dragPreviewLayerRef.current.style.transform =
          offset === 0 ? "" : `translate3d(${offset}px, 0, 0)`;
    });
  }, [dragPreviewLayerRef, dragRef]);

  useEffect(
    () => () => {
      if (previewFrameRef.current != null) window.cancelAnimationFrame(previewFrameRef.current);
    },
    [],
  );

  return { resetPreview, schedulePreview };
}

function useConversationActivityDragEvents({
  buckets,
  viewport,
  setViewport,
  interactionRef,
  dragRef,
  resetPreview,
  schedulePreview,
}: {
  buckets: ConversationActivityBucket[];
  viewport: ConversationActivityViewport;
  setViewport: Dispatch<SetStateAction<ConversationActivityViewport>>;
  interactionRef: MutableRefObject<HTMLDivElement | null>;
  dragRef: MutableRefObject<ConversationActivityDragState | null>;
  resetPreview: () => void;
  schedulePreview: () => void;
}) {
  const handlePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0 || buckets.length <= CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS) return;
      resetPreview();
      dragRef.current = {
        pointerId: event.pointerId,
        startClientX: event.clientX,
        startClientY: event.clientY,
        currentClientX: event.clientX,
        currentClientY: event.clientY,
        axis: "pending",
        viewport: normalizeConversationActivityViewport(viewport, buckets.length),
      };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [buckets.length, dragRef, resetPreview, viewport],
  );

  const handlePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      drag.currentClientX = event.clientX;
      drag.currentClientY = event.clientY;
      if (drag.axis === "pending") {
        const deltaX = Math.abs(drag.currentClientX - drag.startClientX);
        const deltaY = Math.abs(drag.currentClientY - drag.startClientY);
        if (Math.hypot(deltaX, deltaY) < CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX)
          return;
        if (deltaX >= deltaY * CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "horizontal";
        } else if (deltaY >= deltaX * CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO) {
          dragRef.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
          return;
        } else if (
          Math.min(deltaX, deltaY) >=
          Math.max(deltaX, deltaY) * CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO
        ) {
          drag.axis = "free";
        } else {
          return;
        }
      }
      schedulePreview();
    },
    [dragRef, schedulePreview],
  );

  const handlePointerEnd = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (drag.axis === "horizontal" || drag.axis === "free") {
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        const span = drag.viewport.endIndex - drag.viewport.startIndex + 1;
        const delta = Math.round(
          ((drag.startClientX - drag.currentClientX) / Math.max(1, width)) * span,
        );
        setViewport((current) => {
          const next = shiftConversationActivityViewport(drag.viewport, buckets.length, delta);
          return isSameConversationActivityViewport(
            normalizeConversationActivityViewport(current, buckets.length),
            next,
          )
            ? current
            : next;
        });
      }
      dragRef.current = null;
      resetPreview();
      if (event.currentTarget.hasPointerCapture(event.pointerId))
        event.currentTarget.releasePointerCapture(event.pointerId);
    },
    [buckets.length, dragRef, interactionRef, resetPreview, setViewport],
  );

  return { handlePointerDown, handlePointerMove, handlePointerEnd };
}

export function useConversationActivityDrag(
  buckets: ConversationActivityBucket[],
  viewport: ConversationActivityViewport,
  setViewport: Dispatch<SetStateAction<ConversationActivityViewport>>,
) {
  const interactionRef = useRef<HTMLDivElement | null>(null);
  const dragPreviewLayerRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<ConversationActivityDragState | null>(null);
  const preview = useConversationActivityDragPreview(dragRef, dragPreviewLayerRef);
  const handlers = useConversationActivityDragEvents({
    buckets,
    viewport,
    setViewport,
    interactionRef,
    dragRef,
    ...preview,
  });

  return { interactionRef, dragPreviewLayerRef, ...handlers };
}
