import {
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'

export interface InlineChartInteractionState {
  activeIndex: number | null
  isPinned: boolean
  isOpen: boolean
}

export interface TooltipAnchor {
  x: number
  y: number
}

type ChartItemElement = HTMLElement | SVGElement

export interface InlineChartItemProps {
  ref: (node: ChartItemElement | null) => void
  onPointerEnter: (event: ReactPointerEvent<ChartItemElement>) => void
  onPointerMove: (event: ReactPointerEvent<ChartItemElement>) => void
  onPointerDown: (event: ReactPointerEvent<ChartItemElement>) => void
  onMouseEnter: (event: ReactMouseEvent<ChartItemElement>) => void
  onMouseMove: (event: ReactMouseEvent<ChartItemElement>) => void
  onMouseDown: () => void
  onTouchStart: () => void
  onClick: () => void
  'data-inline-chart-index': number
}

interface UseInlineChartInteractionOptions {
  itemCount: number
  defaultIndex: number
}

interface InlineChartInteractionApi {
  containerRef: RefObject<HTMLDivElement | null>
  tooltipRef: RefObject<HTMLDivElement | null>
  state: InlineChartInteractionState
  anchor: TooltipAnchor | null
  getContainerProps: (options: {
    ariaLabel: string
    describedBy?: string
    onBlur?: (event: FocusEvent<HTMLDivElement>) => void
  }) => {
    tabIndex: number
    role: 'group'
    'aria-label': string
    'aria-describedby'?: string
    onFocus: () => void
    onBlur: (event: FocusEvent<HTMLDivElement>) => void
    onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void
    onPointerLeave: (event: ReactPointerEvent<HTMLDivElement>) => void
    onMouseLeave: () => void
  }
  getItemProps: (index: number) => InlineChartItemProps
}

export function useInlineChartInteraction({ itemCount, defaultIndex }: UseInlineChartInteractionOptions): InlineChartInteractionApi {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const tooltipRef = useRef<HTMLDivElement | null>(null)
  const itemRefs = useRef<Array<ChartItemElement | null>>([])
  const lastPointerTypeRef = useRef<string | null>(null)
  const [state, setState] = useState<InlineChartInteractionState>({
    activeIndex: null,
    isPinned: false,
    isOpen: false,
  })
  const [anchor, setAnchor] = useState<TooltipAnchor | null>(null)

  useEffect(() => {
    itemRefs.current.length = itemCount
    if (state.activeIndex != null && state.activeIndex >= itemCount) {
      setState({ activeIndex: null, isPinned: false, isOpen: false })
      setAnchor(null)
    }
  }, [itemCount, state.activeIndex])

  const resolveAnchorFromClientPoint = useCallback((clientX: number, clientY: number): TooltipAnchor | null => {
    const container = containerRef.current
    if (!container) return null
    const rect = container.getBoundingClientRect()
    return {
      x: clientX - rect.left,
      y: clientY - rect.top,
    }
  }, [])

  const resolveAnchorFromItem = useCallback((index: number): TooltipAnchor | null => {
    const container = containerRef.current
    const item = itemRefs.current[index]
    if (!container || !item) return null
    const containerRect = container.getBoundingClientRect()
    const itemRect = item.getBoundingClientRect()
    return {
      x: itemRect.left - containerRect.left + itemRect.width / 2,
      y: itemRect.top - containerRect.top + itemRect.height / 2,
    }
  }, [])

  const openAtIndex = useCallback(
    (index: number, nextAnchor: TooltipAnchor | null, isPinned: boolean) => {
      if (index < 0 || index >= itemCount) return
      const resolvedAnchor = nextAnchor ?? resolveAnchorFromItem(index)
      if (!resolvedAnchor) return
      setState({ activeIndex: index, isPinned, isOpen: true })
      setAnchor(resolvedAnchor)
    },
    [itemCount, resolveAnchorFromItem],
  )

  const close = useCallback(() => {
    setState({ activeIndex: null, isPinned: false, isOpen: false })
    setAnchor(null)
  }, [])

  useEffect(() => {
    if (!state.isPinned) return undefined

    const handlePointerDown = (event: PointerEvent) => {
      const container = containerRef.current
      if (!container) return
      if (container.contains(event.target as Node)) return
      close()
    }

    document.addEventListener('pointerdown', handlePointerDown)
    return () => document.removeEventListener('pointerdown', handlePointerDown)
  }, [close, state.isPinned])

  const handleFocus = useCallback(() => {
    if (itemCount === 0 || state.isOpen) return
    openAtIndex(defaultIndex, resolveAnchorFromItem(defaultIndex), false)
  }, [defaultIndex, itemCount, openAtIndex, resolveAnchorFromItem, state.isOpen])

  const handleBlur = useCallback(
    (event: FocusEvent<HTMLDivElement>, onBlur?: (event: FocusEvent<HTMLDivElement>) => void) => {
      onBlur?.(event)
      const currentTarget = event.currentTarget
      const nextTarget = event.relatedTarget
      if (nextTarget instanceof Node && currentTarget.contains(nextTarget)) return
      close()
    },
    [close],
  )

  const moveByKeyboard = useCallback(
    (nextIndex: number) => {
      const safeIndex = Math.max(0, Math.min(itemCount - 1, nextIndex))
      openAtIndex(safeIndex, resolveAnchorFromItem(safeIndex), false)
    },
    [itemCount, openAtIndex, resolveAnchorFromItem],
  )

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (itemCount === 0) return
      const currentIndex = state.activeIndex ?? defaultIndex
      if (event.key === 'ArrowLeft') {
        event.preventDefault()
        moveByKeyboard(currentIndex - 1)
        return
      }
      if (event.key === 'ArrowRight') {
        event.preventDefault()
        moveByKeyboard(currentIndex + 1)
        return
      }
      if (event.key === 'Home') {
        event.preventDefault()
        moveByKeyboard(0)
        return
      }
      if (event.key === 'End') {
        event.preventDefault()
        moveByKeyboard(itemCount - 1)
        return
      }
      if (event.key === 'Escape') {
        event.preventDefault()
        close()
      }
    },
    [close, defaultIndex, itemCount, moveByKeyboard, state.activeIndex],
  )

  const registerItem = useCallback(
    (index: number) => (node: ChartItemElement | null) => {
      itemRefs.current[index] = node
    },
    [],
  )

  const getItemProps = useCallback(
    (index: number): InlineChartItemProps => ({
      ref: registerItem(index),
      onPointerEnter: (event) => {
        if (event.pointerType !== 'mouse' || state.isPinned) return
        openAtIndex(index, resolveAnchorFromClientPoint(event.clientX, event.clientY), false)
      },
      onPointerMove: (event) => {
        if (event.pointerType !== 'mouse' || state.isPinned) return
        openAtIndex(index, resolveAnchorFromClientPoint(event.clientX, event.clientY), false)
      },
      onPointerDown: (event) => {
        lastPointerTypeRef.current = event.pointerType
      },
      onMouseEnter: (event) => {
        if (state.isPinned) return
        openAtIndex(index, resolveAnchorFromClientPoint(event.clientX, event.clientY), false)
      },
      onMouseMove: (event) => {
        if (state.isPinned) return
        openAtIndex(index, resolveAnchorFromClientPoint(event.clientX, event.clientY), false)
      },
      onMouseDown: () => {
        lastPointerTypeRef.current = 'mouse'
      },
      onTouchStart: () => {
        lastPointerTypeRef.current = 'touch'
      },
      onClick: () => {
        const pointerType = lastPointerTypeRef.current
        if (!pointerType || pointerType === 'mouse') return
        if (state.isPinned && state.activeIndex === index) {
          close()
          return
        }
        openAtIndex(index, resolveAnchorFromItem(index), true)
      },
      'data-inline-chart-index': index,
    }),
    [close, openAtIndex, registerItem, resolveAnchorFromClientPoint, resolveAnchorFromItem, state.activeIndex, state.isPinned],
  )

  const getContainerProps = useCallback(
    ({ ariaLabel, describedBy, onBlur }: { ariaLabel: string; describedBy?: string; onBlur?: (event: FocusEvent<HTMLDivElement>) => void }) => ({
      tabIndex: itemCount > 0 ? 0 : -1,
      role: 'group' as const,
      'aria-label': ariaLabel,
      'aria-describedby': describedBy,
      onFocus: handleFocus,
      onBlur: (event: FocusEvent<HTMLDivElement>) => handleBlur(event, onBlur),
      onKeyDown: handleKeyDown,
      onPointerLeave: (event: ReactPointerEvent<HTMLDivElement>) => {
        if (state.isPinned || event.pointerType !== 'mouse') return
        close()
      },
      onMouseLeave: () => {
        if (state.isPinned) return
        close()
      },
    }),
    [close, handleBlur, handleFocus, handleKeyDown, itemCount, state.isPinned],
  )

  return {
    containerRef,
    tooltipRef,
    state,
    anchor,
    getContainerProps,
    getItemProps,
  }
}
