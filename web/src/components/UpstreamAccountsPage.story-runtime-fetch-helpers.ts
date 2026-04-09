import type {
  ImportOauthCredentialFilePayload,
  ImportedOauthValidationResponse,
  ImportedOauthValidationRow,
} from '../lib/api'
import {
  buildBulkSyncRowEvent,
  buildBulkSyncSnapshotEvent,
  clone,
  storyFutureExpiresAt,
  type StoryStore,
  updateStoryBulkSyncJob,
} from './UpstreamAccountsPage.story-runtime-core'

export function jsonResponse(payload: unknown, status = 200) {
  return Promise.resolve(
    new Response(JSON.stringify(payload), {
      status,
      headers: { 'Content-Type': 'application/json' },
    }),
  )
}

export function wait(ms: number) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms)
  })
}

export function getRosterResponseDelay(storyId: string | null, url: URL) {
  if (storyId?.endsWith('--slow-filter-switch') === true) {
    const workStatuses = url.searchParams.getAll('workStatus')
    if (workStatuses.includes('rate_limited')) {
      return 1_200
    }
  }

  if (storyId?.endsWith('--slow-page-switch') === true) {
    const page = Number(url.searchParams.get('page') || 1)
    if (page === 2) {
      return 1_200
    }
  }

  if (storyId?.endsWith('--current-query-failure') === true) {
    const workStatuses = url.searchParams.getAll('workStatus')
    if (workStatuses.includes('rate_limited')) {
      return 200
    }
  }

  return 0
}

export function getRosterResponseFailure(storyId: string | null, url: URL) {
  if (storyId?.endsWith('--current-query-failure') === true) {
    const workStatuses = url.searchParams.getAll('workStatus')
    if (workStatuses.includes('rate_limited')) {
      return 'storybook forced roster query failure'
    }
  }

  return null
}

export function noContent() {
  return Promise.resolve(new Response(null, { status: 204 }))
}

export function parseBody<T>(raw: BodyInit | null | undefined, fallback: T): T {
  if (typeof raw !== 'string' || !raw) return fallback
  try {
    return JSON.parse(raw) as T
  } catch {
    return fallback
  }
}

function buildStoryImportedOauthValidationRow(
  item: ImportOauthCredentialFilePayload,
): ImportedOauthValidationRow {
  let parsed: unknown
  try {
    parsed = JSON.parse(item.content)
  } catch {
    return {
      sourceId: item.sourceId,
      fileName: item.fileName,
      email: null,
      chatgptAccountId: null,
      displayName: null,
      tokenExpiresAt: null,
      matchedAccount: null,
      status: 'invalid',
      detail: 'Mock validation requires valid JSON.',
      attempts: 1,
    }
  }

  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return {
      sourceId: item.sourceId,
      fileName: item.fileName,
      email: null,
      chatgptAccountId: null,
      displayName: null,
      tokenExpiresAt: null,
      matchedAccount: null,
      status: 'invalid',
      detail: 'Mock validation expects exactly one JSON object.',
      attempts: 1,
    }
  }

  const candidate = parsed as Record<string, unknown>
  const email = typeof candidate.email === 'string' ? candidate.email.trim() : ''
  const chatgptAccountId =
    typeof candidate.account_id === 'string' ? candidate.account_id.trim() : ''
  const forcedStatus =
    typeof candidate._storybookStatus === 'string' ? candidate._storybookStatus : null
  const detail =
    typeof candidate._storybookDetail === 'string' ? candidate._storybookDetail : null

  if (!email || !chatgptAccountId) {
    return {
      sourceId: item.sourceId,
      fileName: item.fileName,
      email: email || null,
      chatgptAccountId: chatgptAccountId || null,
      displayName: email || null,
      tokenExpiresAt: null,
      matchedAccount: null,
      status: 'invalid',
      detail: detail ?? 'Mock validation requires both email and account_id.',
      attempts: 1,
    }
  }

  return {
    sourceId: item.sourceId,
    fileName: item.fileName,
    email,
    chatgptAccountId,
    displayName: email,
    tokenExpiresAt:
      typeof candidate.expired === 'string' && candidate.expired.trim()
        ? candidate.expired.trim()
        : storyFutureExpiresAt,
    matchedAccount: null,
    status:
      forcedStatus ??
      (item.fileName.toLowerCase().includes('exhausted') ? 'ok_exhausted' : 'ok'),
    detail,
    attempts: 1,
  }
}

export function buildStoryImportedOauthValidationResponse(
  items: ImportOauthCredentialFilePayload[],
): ImportedOauthValidationResponse {
  const rows = items.map((item) => buildStoryImportedOauthValidationRow(item))
  const seenKeys = new Set<string>()
  const dedupedRows = rows.map((row) => {
    if (row.status === 'pending') return row
    const normalizedAccountId = row.chatgptAccountId?.trim().toLowerCase()
    const normalizedEmail = row.email?.trim().toLowerCase()
    const matchKey = normalizedAccountId
      ? `account:${normalizedAccountId}`
      : normalizedEmail
        ? `email:${normalizedEmail}`
        : null
    if (!matchKey) return row
    if (seenKeys.has(matchKey)) {
      return {
        ...row,
        matchedAccount: null,
        status: 'duplicate_in_input',
        detail: 'duplicate credential in current import selection',
      }
    }
    seenKeys.add(matchKey)
    return row
  })
  const duplicateInInput = dedupedRows.filter((row) => row.status === 'duplicate_in_input').length
  return {
    inputFiles: items.length,
    uniqueInInput: Math.max(0, dedupedRows.length - duplicateInInput),
    duplicateInInput,
    rows: dedupedRows,
  }
}

export class MockStoryBulkSyncEventSource implements EventTarget {
  private listeners = new Map<string, Set<EventListener>>()
  private timers: number[] = []
  private readonly storeRef: { current: StoryStore }
  readyState = 1
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null

  constructor(storeRef: { current: StoryStore }, url: string | URL) {
    this.storeRef = storeRef
    this.bootstrap(url.toString())
  }

  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
  ) {
    if (!listener) return
    const handler =
      typeof listener === 'function'
        ? listener
        : ((event: Event) => listener.handleEvent(event)) as EventListener
    const current = this.listeners.get(type) ?? new Set<EventListener>()
    current.add(handler)
    this.listeners.set(type, current)
  }

  removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
  ) {
    if (!listener) return
    const current = this.listeners.get(type)
    if (!current) return
    const handler =
      typeof listener === 'function'
        ? listener
        : ((event: Event) => listener.handleEvent(event)) as EventListener
    current.delete(handler)
    if (current.size === 0) {
      this.listeners.delete(type)
    }
  }

  dispatchEvent(event: Event): boolean {
    const current = Array.from(this.listeners.get(event.type) ?? [])
    current.forEach((listener) => listener(event))
    return true
  }

  close() {
    this.readyState = 2
    this.timers.forEach((timer) => window.clearTimeout(timer))
    this.timers = []
    this.listeners.clear()
  }

  private emit(type: string, payload: unknown) {
    if (this.readyState === 2) return
    this.dispatchEvent(
      new MessageEvent(type, {
        data: JSON.stringify(payload),
      }),
    )
  }

  private schedule(delayMs: number, callback: () => void) {
    const timer = window.setTimeout(() => {
      if (this.readyState === 2) return
      callback()
    }, delayMs)
    this.timers.push(timer)
  }

  private bootstrap(rawUrl: string) {
    const parsed = new URL(rawUrl, window.location.origin)
    const match = parsed.pathname.match(
      /^\/api\/pool\/upstream-accounts\/bulk-sync-jobs\/([^/]+)\/events$/,
    )
    if (!match) return

    const jobId = decodeURIComponent(match[1])
    const store = this.storeRef.current
    const job = store.bulkSyncJobs[jobId]
    if (!job) return

    this.schedule(0, () => {
      this.emit('snapshot', buildBulkSyncSnapshotEvent(job))
    })

    if (store.bulkSyncScenario === 'success-auto-hide') {
      this.schedule(80, () => {
        const nextRows = job.snapshot.rows.map((row) => ({
          ...row,
          status: 'succeeded',
          detail: null,
        }))
        const lastRow = clone(nextRows[nextRows.length - 1])
        updateStoryBulkSyncJob(job, nextRows, 'running')
        this.emit('row', buildBulkSyncRowEvent(lastRow, job.counts))
      })
      this.schedule(160, () => {
        updateStoryBulkSyncJob(job, job.snapshot.rows, 'completed')
        this.emit('completed', buildBulkSyncSnapshotEvent(job))
      })
      return
    }

    if (store.bulkSyncScenario === 'partial-failure') {
      this.schedule(80, () => {
        const nextRows = job.snapshot.rows.map((row, index) =>
          index === 0
            ? {
                ...row,
                status: 'failed',
                detail: 'refresh token already rotated',
              }
            : {
                ...row,
                status: 'succeeded',
                detail: null,
              },
        )
        const firstRow = clone(nextRows[0])
        updateStoryBulkSyncJob(job, nextRows, 'running')
        this.emit('row', buildBulkSyncRowEvent(firstRow, job.counts))
      })
      this.schedule(160, () => {
        updateStoryBulkSyncJob(job, job.snapshot.rows, 'completed')
        this.emit('completed', buildBulkSyncSnapshotEvent(job))
      })
    }
  }
}
