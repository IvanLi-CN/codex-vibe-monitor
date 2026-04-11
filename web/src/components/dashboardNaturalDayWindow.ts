const MINUTE_MS = 60_000

export interface NaturalDayWindowLike {
  rangeStart?: string | null
  rangeEnd?: string | null
  bucketSeconds?: number | null
}

export function parseDateInput(value?: string | null) {
  if (!value) return null
  if (value.includes('T')) {
    const parsed = new Date(value)
    return Number.isNaN(parsed.getTime()) ? null : parsed
  }

  const [datePart, timePart] = value.split(' ')
  const [year, month, day] = (datePart ?? '').split('-').map(Number)
  const [hour, minute, second] = (timePart ?? '').split(':').map(Number)
  if (![year, month, day].every(Number.isFinite)) return null
  const parsed = new Date(
    year,
    Math.max(0, month - 1),
    day,
    Number.isFinite(hour) ? hour : 0,
    Number.isFinite(minute) ? minute : 0,
    Number.isFinite(second) ? second : 0,
    0,
  )
  return Number.isNaN(parsed.getTime()) ? null : parsed
}

export function resolveClosedNaturalDayEnd(
  response?: NaturalDayWindowLike | null,
) {
  const rangeStart = parseDateInput(response?.rangeStart)
  const rangeEnd = parseDateInput(response?.rangeEnd)
  if (!rangeStart || !rangeEnd || !isLocalMidnight(rangeStart)) {
    return null
  }

  const closedDayEnd = startOfLocalDay(rangeEnd)
  const endOffsetMs = rangeEnd.getTime() - closedDayEnd.getTime()
  const bucketMs = Math.max((response?.bucketSeconds ?? 0) * 1000, MINUTE_MS)
  const durationMs = rangeEnd.getTime() - rangeStart.getTime()

  if (durationMs < 23 * 60 * MINUTE_MS) {
    return null
  }
  if (endOffsetMs < 0 || endOffsetMs > bucketMs) {
    return null
  }

  return closedDayEnd
}

function startOfLocalDay(date: Date) {
  const next = new Date(date)
  next.setHours(0, 0, 0, 0)
  return next
}

function isLocalMidnight(date: Date) {
  return (
    date.getHours() === 0 &&
    date.getMinutes() === 0 &&
    date.getSeconds() === 0 &&
    date.getMilliseconds() === 0
  )
}
