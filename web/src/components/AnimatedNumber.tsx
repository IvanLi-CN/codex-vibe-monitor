import { useEffect, useMemo, useRef, useState } from 'react'

interface AnimatedNumberProps {
  value: number | string
  duration?: number
  easing?: string
  className?: string
}

/**
 * Whole-string vertical slide animation for numeric text.
 * Always animates as a single block in the direction of the overall numeric delta.
 */
export function AnimatedNumber({ value, duration = 450, easing = 'cubic-bezier(0.22, 1, 0.36, 1)', className }: AnimatedNumberProps) {
  const nextText = String(value ?? '')
  const prevRef = useRef(nextText)
  const [stack, setStack] = useState<string[]>([nextText])
  const [index, setIndex] = useState(0)

  const direction: 'up' | 'down' | 'none' = useMemo(() => {
    const prevNum = parseNumber(prevRef.current)
    const nextNum = parseNumber(nextText)
    if (Number.isFinite(prevNum) && Number.isFinite(nextNum)) {
      if (nextNum > prevNum) return 'up'
      if (nextNum < prevNum) return 'down'
    }
    return 'none'
  }, [nextText])

  useEffect(() => {
    if (prevRef.current === nextText) return
    const from = prevRef.current
    const to = nextText
    const items = direction === 'up' ? [from, to] : direction === 'down' ? [to, from] : [to]
    // When increasing, we want the container to slide up (from->to): translateY from 0 to -1em
    // When decreasing, we arrange [to, from] and slide down: from start at 1em to 0
    if (direction === 'up') {
      setStack(items)
      setIndex(0)
      requestAnimationFrame(() => setIndex(1))
    } else if (direction === 'down') {
      setStack(items)
      setIndex(1)
      requestAnimationFrame(() => setIndex(0))
    } else {
      setStack([to])
      setIndex(0)
    }
    prevRef.current = nextText
  }, [direction, nextText])

  const translateY = useMemo(() => {
    if (stack.length === 1) return 0
    if (direction === 'up') {
      // [from, to] index: 0 -> 1 : 0 -> -1em
      return -index * 1.0
    }
    // direction === 'down': [to, from] index: 1 -> 0 : -1em -> 0
    return index === 1 ? -1 : 0
  }, [direction, index, stack.length])

  const hasTransition = stack.length > 1

  return (
    <span className={className} style={{ display: 'inline-block', height: '1em', lineHeight: 1, overflow: 'hidden', fontVariantNumeric: 'tabular-nums' }}>
      <span
        style={{
          display: 'inline-flex',
          flexDirection: 'column',
          transform: `translateY(${translateY}em)`,
          transition: hasTransition ? `transform ${duration}ms ${easing}` : 'none',
          willChange: hasTransition ? 'transform' : 'auto',
        }}
      >
        {stack.map((line, i) => (
          <span key={`${i}-${line}`} style={{ height: '1em' }}>
            {line}
          </span>
        ))}
      </span>
    </span>
  )
}

function parseNumber(s: string): number {
  const cleaned = s.replace(/[^0-9.\-]/g, '') // eslint-disable-line no-useless-escape
  const parts = cleaned.split('.')
  const normalized = parts.length > 2 ? `${parts[0]}.${parts.slice(1).join('')}` : cleaned
  const n = Number(normalized)
  return Number.isFinite(n) ? n : NaN
}

export default AnimatedNumber
