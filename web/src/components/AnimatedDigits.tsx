import { useEffect, useMemo, useRef, useState } from 'react'

interface AnimatedDigitsProps {
  value: number | string
  duration?: number
  easing?: string
  className?: string
}

type Direction = 'up' | 'down' | 'none'

export function AnimatedDigits({ value, duration = 450, easing = 'cubic-bezier(0.22, 1, 0.36, 1)', className }: AnimatedDigitsProps) {
  const text = String(value ?? '')

  // Track previous text to compute direction and per-position transitions
  const [prevText, setPrevText] = useState(text)

  const direction: Direction = useMemo(() => {
    const prevNum = parseNumber(prevText)
    const nextNum = parseNumber(text)
    if (Number.isFinite(prevNum) && Number.isFinite(nextNum)) {
      if (nextNum > prevNum) return 'up'
      if (nextNum < prevNum) return 'down'
    }
    return 'none'
  }, [prevText, text])

  const mapping = useMemo(() => buildDigitMapping(prevText, text), [prevText, text])

  useEffect(() => {
    // update prev after rendering with new text, so next change compares correctly
    setPrevText(text)
  }, [text])

  let digitIndex = 0
  return (
    <span className={className} style={{ display: 'inline-flex', alignItems: 'baseline', gap: '0' }}>
      {text.split('').map((ch, idx) => {
        if (/\d/.test(ch)) {
          const prev = mapping.prevDigitsAligned[digitIndex] ?? clampDigit(ch)
          const next = clampDigit(ch)
          digitIndex += 1
          return (
            <DigitRoll key={idx} prev={prev} next={next} direction={direction} duration={duration} easing={easing} />
          )
        }
        return (
          <span key={idx}>{ch}</span>
        )
      })}
    </span>
  )
}

function DigitRoll({ prev, next, direction, duration, easing }: { prev: number; next: number; direction: Direction; duration: number; easing: string }) {
  const [path, setPath] = useState<number[]>([next])
  const [index, setIndex] = useState<number>(path.length - 1)
  const firstRender = useRef(true)

  useEffect(() => {
    const newPath = buildPath(prev, next, direction)
    setPath(newPath)
    // Kick the transition on the next frame
    requestAnimationFrame(() => setIndex(newPath.length - 1))
  }, [prev, next, direction])

  useEffect(() => {
    if (firstRender.current) {
      firstRender.current = false
      return
    }
  }, [])

  const translateY = -index * 1.0
  const hasTransition = path.length > 1

  return (
    <span
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
          transition: hasTransition ? `transform ${duration}ms ${easing}` : 'none',
          willChange: hasTransition ? 'transform' : 'auto',
        }}
      >
        {path.map((d, i) => (
          <span key={`${d}-${i}`} style={{ height: '1em' }}>
            {d}
          </span>
        ))}
      </span>
    </span>
  )
}

function buildPath(prev: number, next: number, dir: Direction): number[] {
  if (dir === 'none' || prev === next) return [next]
  const seq: number[] = [prev]
  if (dir === 'up') {
    let cur = prev
    while (cur !== next) {
      cur = (cur + 1) % 10
      seq.push(cur)
      if (seq.length > 12) break // safety
    }
  } else {
    let cur = prev
    while (cur !== next) {
      cur = (cur + 9) % 10 // minus 1 mod 10
      seq.push(cur)
      if (seq.length > 12) break // safety
    }
  }
  return seq
}

function buildDigitMapping(prevText: string, nextText: string) {
  const prevDigits = extractDigits(prevText)
  const nextDigits = extractDigits(nextText)
  const maxLen = Math.max(prevDigits.length, nextDigits.length)
  const prevPadded = padLeft(prevDigits, maxLen, 0)
  // Align from right: reverse mapping then reverse back
  const prevRev = prevPadded.slice().reverse()
  // We return prev aligned sequence in normal (left-to-right) order for next digits sequence
  const alignedRev: number[] = []
  for (let i = 0; i < maxLen; i++) {
    alignedRev.push(prevRev[i] ?? 0)
  }
  const prevDigitsAligned = alignedRev.slice().reverse()
  return { prevDigitsAligned }
}

function extractDigits(s: string): number[] {
  const arr: number[] = []
  for (const ch of s) {
    if (/\d/.test(ch)) arr.push(clampDigit(ch))
  }
  return arr
}

function padLeft<T>(arr: T[], len: number, fill: T): T[] {
  if (arr.length >= len) return arr
  return Array(len - arr.length).fill(fill).concat(arr)
}

function parseNumber(s: string): number {
  // keep digits and at most one decimal point
  const cleaned = s.replace(/[^0-9.]/g, '')
  const parts = cleaned.split('.')
  const normalized = parts.length > 2 ? `${parts[0]}.${parts.slice(1).join('')}` : cleaned
  const n = Number(normalized)
  return Number.isFinite(n) ? n : NaN
}

const DIGIT_ZERO_CODE = '0'.charCodeAt(0)
function clampDigit(ch: string): number {
  const code = ch.charCodeAt(0)
  if (code >= DIGIT_ZERO_CODE && code <= DIGIT_ZERO_CODE + 9) return code - DIGIT_ZERO_CODE
  return 0
}

export default AnimatedDigits
