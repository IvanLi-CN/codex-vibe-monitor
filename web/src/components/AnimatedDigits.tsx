import { useEffect, useMemo, useRef, useState } from 'react'

interface AnimatedDigitsProps {
  value: number | string
  /**
   * Duration of the rolling animation in milliseconds
   */
  duration?: number
  /**
   * Easing function for CSS transitions
   */
  easing?: string
  /** Optional className applied to the root container */
  className?: string
}

/**
 * AnimatedDigits renders a string or number and animates numeric characters by vertically
 * rolling them to the next value. Non-digit characters are rendered statically.
 */
export function AnimatedDigits({ value, duration = 450, easing = 'cubic-bezier(0.22, 1, 0.36, 1)', className }: AnimatedDigitsProps) {
  const text = String(value ?? '')
  const tokens = useMemo(() => text.split(''), [text])
  return (
    <span className={className} style={{ display: 'inline-flex', alignItems: 'baseline', gap: '0' }}>
      {tokens.map((ch, idx) =>
        /\d/.test(ch) ? (
          <DigitRoll key={idx} digit={ch} duration={duration} easing={easing} />
        ) : (
          <span key={idx}>{ch}</span>
        ),
      )}
    </span>
  )
}

function DigitRoll({ digit, duration, easing }: { digit: string; duration: number; easing: string }) {
  const target = clampDigit(digit)
  const [current, setCurrent] = useState<number>(target)
  const containerRef = useRef<HTMLSpanElement | null>(null)

  useEffect(() => {
    const next = clampDigit(digit)
    setCurrent((prev) => (prev === next ? prev : next))
  }, [digit])

  const translateY = -current * 1.0 // 1em per line

  return (
    <span
      ref={containerRef}
      aria-hidden={false}
      style={{
        display: 'inline-block',
        height: '1em',
        lineHeight: 1,
        overflow: 'hidden',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      <span
        style={{
          display: 'inline-flex',
          flexDirection: 'column',
          transform: `translateY(${translateY}em)`,
          transition: `transform ${duration}ms ${easing}`,
          willChange: 'transform',
        }}
      >
        {DIGITS.map((d) => (
          <span key={d} style={{ height: '1em' }}>
            {d}
          </span>
        ))}
      </span>
    </span>
  )
}

const DIGITS = Array.from({ length: 10 }, (_, i) => String(i))

function clampDigit(ch: string): number {
  const code = ch.charCodeAt(0)
  if (code >= 48 && code <= 57) return code - 48
  return 0
}

export default AnimatedDigits

