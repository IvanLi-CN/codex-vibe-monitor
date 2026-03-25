import { type ReactNode, useEffect, useId, useLayoutEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { cn } from '../../lib/utils'
import { useInlineChartInteraction } from './use-inline-chart-interaction'
import { floatingSurfaceStyle } from './floating-surface'
import { usePortaledTheme } from './use-portaled-theme'

const TOOLTIP_OFFSET = 12
const TOOLTIP_PADDING = 8

type TooltipTone = 'neutral' | 'success' | 'error' | 'accent'

interface TooltipPosition {
  x: number
  y: number
  placementX: 'left' | 'right'
  placementY: 'top' | 'bottom'
}

export interface InlineChartTooltipRow {
  label: string
  value: string
  tone?: TooltipTone
}

export interface InlineChartTooltipData {
  title: string
  rows: InlineChartTooltipRow[]
}

interface InlineChartTooltipSurfaceProps {
  items: InlineChartTooltipData[]
  defaultIndex: number
  ariaLabel: string
  interactionHint: string
  linkedActiveIndex?: number | null
  onActiveIndexChange?: (index: number | null) => void
  className?: string
  chartClassName?: string
  children: (api: {
    activeIndex: number | null
    highlightedIndex: number | null
    getItemProps: ReturnType<typeof useInlineChartInteraction>['getItemProps']
  }) => ReactNode
}

function toneClasses(tone: TooltipTone | undefined) {
  switch (tone) {
    case 'success':
      return 'bg-success/80'
    case 'error':
      return 'bg-error/80'
    case 'accent':
      return 'bg-primary/80'
    default:
      return 'bg-base-content/30'
  }
}

function serializeTooltipForAssistiveTech(tooltip: InlineChartTooltipData | null) {
  if (!tooltip) return null
  return [tooltip.title, ...tooltip.rows.map((row) => `${row.label} ${row.value}`)].join(', ')
}

export function InlineChartTooltipSurface({
  items,
  defaultIndex,
  ariaLabel,
  interactionHint,
  linkedActiveIndex = null,
  onActiveIndexChange,
  className,
  chartClassName,
  children,
}: InlineChartTooltipSurfaceProps) {
  const hintId = useId()
  const tooltipId = useId()
  const liveRegionId = useId()
  const { containerRef, tooltipRef, state, anchor, getContainerProps, getItemProps } = useInlineChartInteraction({
    itemCount: items.length,
    defaultIndex,
  })
  const [position, setPosition] = useState<TooltipPosition | null>(null)
  const portalTheme = usePortaledTheme(containerRef.current)

  const activeTooltip = useMemo(() => {
    if (state.activeIndex == null) return null
    return items[state.activeIndex] ?? null
  }, [items, state.activeIndex])
  const activeTooltipAnnouncement = useMemo(() => serializeTooltipForAssistiveTech(activeTooltip), [activeTooltip])
  const describedBy = useMemo(
    () => [hintId, activeTooltipAnnouncement ? liveRegionId : null].filter(Boolean).join(' '),
    [activeTooltipAnnouncement, hintId, liveRegionId],
  )
  const highlightedIndex = state.isOpen ? state.activeIndex : linkedActiveIndex

  useEffect(() => {
    onActiveIndexChange?.(state.isOpen ? state.activeIndex : null)
  }, [onActiveIndexChange, state.activeIndex, state.isOpen])

  useLayoutEffect(() => {
    if (!state.isOpen || !anchor || !containerRef.current || !tooltipRef.current) {
      setPosition(null)
      return undefined
    }

    const container = containerRef.current
    const tooltip = tooltipRef.current
    const ownerWindow = container.ownerDocument.defaultView ?? window

    const updatePosition = () => {
      const containerRect = container.getBoundingClientRect()
      const tooltipRect = tooltip.getBoundingClientRect()
      const anchorX = containerRect.left + anchor.x
      const anchorY = containerRect.top + anchor.y
      let nextX = anchorX + TOOLTIP_OFFSET
      let nextY = anchorY + TOOLTIP_OFFSET
      let placementX: TooltipPosition['placementX'] = 'right'
      let placementY: TooltipPosition['placementY'] = 'bottom'

      if (nextX + tooltipRect.width > ownerWindow.innerWidth - TOOLTIP_PADDING) {
        nextX = anchorX - tooltipRect.width - TOOLTIP_OFFSET
        placementX = 'left'
      }
      if (nextY + tooltipRect.height > ownerWindow.innerHeight - TOOLTIP_PADDING) {
        nextY = anchorY - tooltipRect.height - TOOLTIP_OFFSET
        placementY = 'top'
      }

      const maxX = Math.max(TOOLTIP_PADDING, ownerWindow.innerWidth - tooltipRect.width - TOOLTIP_PADDING)
      const maxY = Math.max(TOOLTIP_PADDING, ownerWindow.innerHeight - tooltipRect.height - TOOLTIP_PADDING)

      setPosition({
        x: Math.min(Math.max(nextX, TOOLTIP_PADDING), maxX),
        y: Math.min(Math.max(nextY, TOOLTIP_PADDING), maxY),
        placementX,
        placementY,
      })
    }

    updatePosition()
    ownerWindow.addEventListener('resize', updatePosition)
    ownerWindow.addEventListener('scroll', updatePosition, true)

    return () => {
      ownerWindow.removeEventListener('resize', updatePosition)
      ownerWindow.removeEventListener('scroll', updatePosition, true)
    }
  }, [anchor, activeTooltip, containerRef, state.isOpen, tooltipRef])

  const tooltipNode = activeTooltip ? (
    <div
      id={tooltipId}
      ref={tooltipRef}
      data-theme={portalTheme}
      role="tooltip"
      aria-hidden={!position}
      data-inline-chart-tooltip="true"
      data-active-index={state.activeIndex ?? undefined}
      style={{
        ...floatingSurfaceStyle('neutral', portalTheme),
        left: position?.x ?? 0,
        top: position?.y ?? 0,
        transform: 'translateZ(0)',
        visibility: position ? 'visible' : 'hidden',
      }}
      className={cn(
        'pointer-events-none fixed z-[70] min-w-[11rem] max-w-[14rem] rounded-xl border px-3 py-2 text-[11px] leading-tight text-base-content transition-[opacity,transform] duration-150 ease-out motion-reduce:transition-none',
        position?.placementX === 'right' ? 'origin-left' : 'origin-right',
        position?.placementY === 'bottom' ? 'origin-top' : 'origin-bottom',
        position ? 'opacity-100' : 'opacity-0',
      )}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">{activeTooltip.title}</div>
      <dl className="mt-2 space-y-1.5">
        {activeTooltip.rows.map((row) => (
          <div key={`${row.label}-${row.value}`} className="flex items-start gap-2">
            <span className={cn('mt-[5px] h-1.5 w-1.5 shrink-0 rounded-full', toneClasses(row.tone))} aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <dt className="text-base-content/62">{row.label}</dt>
              <dd className="mt-0.5 font-mono text-[12px] font-semibold tracking-tight text-base-content">{row.value}</dd>
            </div>
          </div>
        ))}
      </dl>
    </div>
  ) : null

  const ownerDocument = containerRef.current?.ownerDocument

  return (
    <div
      ref={containerRef}
      className={cn('relative overflow-visible rounded-lg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/70', className)}
      {...getContainerProps({ ariaLabel, describedBy })}
    >
      <span id={hintId} className="sr-only">
        {interactionHint}
      </span>
      {activeTooltipAnnouncement ? (
        <span id={liveRegionId} className="sr-only" aria-live="polite" aria-atomic="true">
          {activeTooltipAnnouncement}
        </span>
      ) : null}
      <div className={cn('relative', chartClassName)}>{children({ activeIndex: state.activeIndex, highlightedIndex, getItemProps })}</div>
      {tooltipNode && ownerDocument
        ? createPortal(
            // Intentional body-root overlay: chart hover tooltips should not inherit local hosts.
            tooltipNode,
            ownerDocument.body,
          )
        : tooltipNode}
    </div>
  )
}
