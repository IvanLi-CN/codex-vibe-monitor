import { useEffect, useRef, type ReactNode } from 'react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { useTheme } from '../theme/context'
import type {
  CreateApiKeyAccountPayload,
  CompleteOauthLoginSessionPayload,
  ImportOauthCredentialFilePayload,
  OauthMailboxStatus,
  UpdateOauthLoginSessionPayload,
  UpdatePoolRoutingSettingsPayload,
  UpdateUpstreamAccountGroupPayload,
  UpdateUpstreamAccountPayload,
  UpstreamAccountListResponse,
} from '../lib/api'
import AccountPoolLayout from '../pages/account-pool/AccountPoolLayout'
import UpstreamAccountCreatePage from '../pages/account-pool/UpstreamAccountCreate'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'

import {
  buildBulkSyncCounts,
  buildBulkSyncSnapshot,
  buildBulkSyncSnapshotEvent,
  clone,
  createBulkSyncRows,
  createApiKeyAccount,
  createStore,
  createOauthAccount,
  currentStoryId,
  defaultRoutingMaintenance,
  defaultRoutingTimeouts,
  filterAccountsForQuery,
  listGroupSummaries,
  listTagSummaries,
  maskApiKey,
  now,
  normalizeGroupName,
  storyFutureExpiresAt,
  storyFutureLoginExpiresAt,
  storyHasExistingMailboxAddress,
  storyHealthStatus,
  storySyncState,
  storyWorkStatus,
  syncLocalWindows,
  toSummary,
  type StoryBulkSyncJob,
  type StoryInitialEntry,
  type StoryStore,
  updateStoryBulkSyncJob,
} from './UpstreamAccountsPage.story-runtime-core'
import {
  buildStickyConversations,
  buildStickyInvocationRecords,
} from './UpstreamAccountsPage.story-runtime-sticky'
import {
  buildStoryImportedOauthValidationResponse,
  getRosterResponseDelay,
  getRosterResponseFailure,
  jsonResponse,
  MockStoryBulkSyncEventSource,
  noContent,
  parseBody,
  wait,
} from './UpstreamAccountsPage.story-runtime-fetch-helpers'

export type { StoryInitialEntry } from './UpstreamAccountsPage.story-runtime-core'

export function StorybookUpstreamAccountsMock({
  children,
}: {
  children: ReactNode
}) {
  const storeRef = useRef<StoryStore>(createStore())
  const originalFetchRef = useRef<typeof window.fetch | null>(null)
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null)
  const installedRef = useRef(false)

  if (typeof window !== 'undefined' && !installedRef.current) {
    installedRef.current = true
    originalFetchRef.current = window.fetch.bind(window)
    originalEventSourceRef.current = window.EventSource

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (
        init?.method || (input instanceof Request ? input.method : 'GET')
      ).toUpperCase()
      const inputUrl =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url
      const parsedUrl = new URL(inputUrl, window.location.origin)
      const path = parsedUrl.pathname
      const storyId = currentStoryId()
      const store = storeRef.current

      if (path === '/api/pool/upstream-accounts' && method === 'GET') {
        const filteredItems = filterAccountsForQuery(store, parsedUrl)
        const rawPageSize = Number(parsedUrl.searchParams.get('pageSize') || 20)
        const requestedPageSize =
          Number.isFinite(rawPageSize) && rawPageSize > 0 ? rawPageSize : 20
        const total = filteredItems.length
        const pageCount = Math.max(1, Math.ceil(total / requestedPageSize))
        const rawPage = Number(parsedUrl.searchParams.get('page') || 1)
        const requestedPage =
          Number.isFinite(rawPage) && rawPage > 0 ? rawPage : 1
        const page = Math.min(requestedPage, pageCount)
        const start = (page - 1) * requestedPageSize
        const pageItems = filteredItems
          .slice(start, start + requestedPageSize)
          .map((item) => clone(item))
        const payload: UpstreamAccountListResponse = {
          writesEnabled: store.writesEnabled,
          groups: listGroupSummaries(store),
          forwardProxyNodes: clone(store.forwardProxyNodes),
          hasUngroupedAccounts: store.accounts.some(
            (account) => !normalizeGroupName(account.groupName),
          ),
          routing: clone(store.routing),
          items: pageItems,
          total,
          page,
          pageSize: requestedPageSize,
          metrics: {
            total,
            oauth: filteredItems.filter((item) => item.kind === 'oauth_codex')
              .length,
            apiKey: filteredItems.filter(
              (item) => item.kind === 'api_key_codex',
            ).length,
            attention: filteredItems.filter((item) => {
              const derivedHealthStatus = storyHealthStatus(item)
              const derivedSyncState = storySyncState(item)
              return (
                derivedHealthStatus !== 'normal' ||
                storyWorkStatus(item, derivedHealthStatus, derivedSyncState) ===
                  'rate_limited'
              )
            }).length,
          },
        }
        const delayMs = getRosterResponseDelay(storyId, parsedUrl)
        const failureMessage = getRosterResponseFailure(storyId, parsedUrl)
        if (delayMs > 0) {
          await wait(delayMs)
        }
        if (failureMessage) {
          return jsonResponse({ message: failureMessage }, 503)
        }
        return jsonResponse(payload)
      }

      if (path === '/api/pool/tags' && method === 'GET') {
        return jsonResponse({
          writesEnabled: store.writesEnabled,
          items: listTagSummaries(store),
        })
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/imports/validate' &&
        method === 'POST'
      ) {
        const body = parseBody<{ items?: ImportOauthCredentialFilePayload[] }>(
          init?.body,
          {},
        )
        const items = Array.isArray(body.items) ? body.items : []
        return jsonResponse(buildStoryImportedOauthValidationResponse(items))
      }

      if (path === '/api/pool/upstream-accounts/bulk-sync-jobs' && method === 'POST') {
        const body = parseBody<{ accountIds?: number[] }>(init?.body, {})
        const accountIds = Array.isArray(body.accountIds) ? body.accountIds : []
        const rows = createBulkSyncRows(store, accountIds)
        const jobId = `story-bulk-sync-${Date.now()}`
        const job: StoryBulkSyncJob = {
          jobId,
          snapshot: buildBulkSyncSnapshot(jobId, rows),
          counts: buildBulkSyncCounts(rows),
          error: null,
        }
        store.bulkSyncJobs[jobId] = job
        return jsonResponse({
          jobId,
          ...buildBulkSyncSnapshotEvent(job),
        }, 201)
      }

      if (
        path.startsWith('/api/pool/upstream-accounts/bulk-sync-jobs/') &&
        method === 'GET'
      ) {
        const match = path.match(
          /^\/api\/pool\/upstream-accounts\/bulk-sync-jobs\/([^/]+)$/,
        )
        if (!match) {
          return jsonResponse({ message: 'not found' }, 404)
        }
        const jobId = decodeURIComponent(match[1])
        const job = store.bulkSyncJobs[jobId]
        if (!job) {
          return jsonResponse({ message: 'not found' }, 404)
        }
        return jsonResponse({
          jobId,
          ...buildBulkSyncSnapshotEvent(job),
        })
      }

      if (
        path.startsWith('/api/pool/upstream-accounts/bulk-sync-jobs/') &&
        method === 'DELETE'
      ) {
        const match = path.match(
          /^\/api\/pool\/upstream-accounts\/bulk-sync-jobs\/([^/]+)$/,
        )
        if (match) {
          const jobId = decodeURIComponent(match[1])
          const job = store.bulkSyncJobs[jobId]
          if (job) {
            updateStoryBulkSyncJob(job, job.snapshot.rows, 'cancelled')
          }
        }
        return noContent()
      }

      if (path === '/api/pool/routing-settings' && method === 'PUT') {
        const body = parseBody<UpdatePoolRoutingSettingsPayload>(init?.body, {})
        const trimmed = body.apiKey?.trim()
        store.routing = {
          ...store.routing,
          ...(trimmed
            ? {
                apiKeyConfigured: true,
                maskedApiKey: maskApiKey(trimmed),
              }
            : {}),
          ...(body.maintenance
            ? {
                maintenance: {
                  primarySyncIntervalSecs:
                    body.maintenance.primarySyncIntervalSecs ??
                    store.routing.maintenance?.primarySyncIntervalSecs ??
                    defaultRoutingMaintenance.primarySyncIntervalSecs,
                  secondarySyncIntervalSecs:
                    body.maintenance.secondarySyncIntervalSecs ??
                    store.routing.maintenance?.secondarySyncIntervalSecs ??
                    defaultRoutingMaintenance.secondarySyncIntervalSecs,
                  priorityAvailableAccountCap:
                    body.maintenance.priorityAvailableAccountCap ??
                    store.routing.maintenance?.priorityAvailableAccountCap ??
                    defaultRoutingMaintenance.priorityAvailableAccountCap,
                },
              }
            : {}),
          ...(body.timeouts
            ? {
                timeouts: {
                  responsesFirstByteTimeoutSecs:
                    body.timeouts.responsesFirstByteTimeoutSecs ??
                    store.routing.timeouts?.responsesFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.responsesFirstByteTimeoutSecs,
                  compactFirstByteTimeoutSecs:
                    body.timeouts.compactFirstByteTimeoutSecs ??
                    store.routing.timeouts?.compactFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.compactFirstByteTimeoutSecs,
                  responsesStreamTimeoutSecs:
                    body.timeouts.responsesStreamTimeoutSecs ??
                    store.routing.timeouts?.responsesStreamTimeoutSecs ??
                    defaultRoutingTimeouts.responsesStreamTimeoutSecs,
                  compactStreamTimeoutSecs:
                    body.timeouts.compactStreamTimeoutSecs ??
                    store.routing.timeouts?.compactStreamTimeoutSecs ??
                    defaultRoutingTimeouts.compactStreamTimeoutSecs,
                },
              }
            : {}),
        }
        return jsonResponse(clone(store.routing))
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/login-sessions' &&
        method === 'POST'
      ) {
        const body = parseBody<{
          displayName?: string
          groupName?: string
          note?: string
          groupNote?: string
          tagIds?: number[]
          isMother?: boolean
          mailboxSessionId?: string
          mailboxAddress?: string
        }>(init?.body, {})
        const loginId = `login_${Date.now()}`
        const redirectUri = `http://localhost:431${String(store.nextId).slice(-1)}/oauth/callback`
        const state = `state_${loginId}`
        const session: StoryStore['sessions'][string] = {
          loginId,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&loginId=${loginId}&state=${state}`,
          redirectUri,
          expiresAt: storyFutureLoginExpiresAt,
          accountId: null,
          error: null,
          displayName: body.displayName,
          groupName: body.groupName,
          isMother: body.isMother,
          note: body.note,
          groupNote: body.groupNote,
          tagIds: Array.isArray(body.tagIds) ? body.tagIds : [],
          mailboxSessionId: body.mailboxSessionId,
          mailboxAddress: body.mailboxAddress,
          state,
        }
        store.sessions[loginId] = session
        return jsonResponse(clone(session), 201)
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/mailbox-sessions' &&
        method === 'POST'
      ) {
        const body = parseBody<{ emailAddress?: string }>(init?.body, {})
        const requestedAddress = body.emailAddress?.trim().toLowerCase() ?? ''
        const shouldDelayMailboxAttach =
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-attach-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-attach-pending' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-attach-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-popover-edit' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-attach-pending'
        const shouldDelayMailboxGenerate =
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-generate-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-oauth--mailbox-generate-pending' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-generate-flow' ||
          storyId ===
            'account-pool-pages-upstream-account-create-batch-oauth--mailbox-generate-pending'
        if (requestedAddress && shouldDelayMailboxAttach) {
          await wait(900)
        }
        if (!requestedAddress && shouldDelayMailboxGenerate) {
          await wait(900)
        }
        if (requestedAddress) {
          if (!requestedAddress.includes('@')) {
            return jsonResponse(
              {
                supported: false,
                emailAddress: requestedAddress,
                reason: 'invalid_format',
              },
              201,
            )
          }
          const isSupportedDomain = requestedAddress.endsWith(
            '@mail-tw.707079.xyz',
          )
          if (!isSupportedDomain) {
            return jsonResponse(
              {
                supported: false,
                emailAddress: requestedAddress,
                reason: 'unsupported_domain',
              },
              201,
            )
          }
          const existingRemoteMailbox = storyHasExistingMailboxAddress(
            store,
            requestedAddress,
          )
          const nextMailboxId = store.nextMailboxId++
          const sessionId = `mailbox_${nextMailboxId}`
          const expiresAt = storyFutureExpiresAt
          store.mailboxStatuses[sessionId] = {
            sessionId,
            emailAddress: requestedAddress,
            expiresAt,
            latestCode: null,
            invite: null,
            invited: false,
          }
          return jsonResponse(
            {
              supported: true,
              sessionId,
              emailAddress: requestedAddress,
              expiresAt,
              source: existingRemoteMailbox ? 'attached' : 'generated',
            },
            201,
          )
        }
        const nextMailboxId = store.nextMailboxId++
        const sessionId = `mailbox_${nextMailboxId}`
        const emailAddress = `storybook-oauth-${nextMailboxId}@mail-tw.707079.xyz`
        const expiresAt = storyFutureExpiresAt
        store.mailboxStatuses[sessionId] = {
          sessionId,
          emailAddress,
          expiresAt,
          latestCode: null,
          invite: null,
          invited: false,
        }
        return jsonResponse(
          {
            supported: true,
            sessionId,
            emailAddress,
            expiresAt,
            source: 'generated',
          },
          201,
        )
      }

      if (
        path === '/api/pool/upstream-accounts/oauth/mailbox-sessions/status' &&
        method === 'POST'
      ) {
        const body = parseBody<{ sessionIds?: string[] }>(init?.body, {})
        const sessionIds = Array.isArray(body.sessionIds) ? body.sessionIds : []
        const items = sessionIds
          .map((sessionId) => store.mailboxStatuses[sessionId])
          .filter((item): item is OauthMailboxStatus => item != null)
        return jsonResponse({ items })
      }

      const mailboxSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/mailbox-sessions\/([^/]+)$/,
      )
      if (mailboxSessionMatch && method === 'DELETE') {
        const sessionId = decodeURIComponent(mailboxSessionMatch[1])
        delete store.mailboxStatuses[sessionId]
        return noContent()
      }

      const loginSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)$/,
      )
      if (loginSessionMatch && method === 'PATCH') {
        const loginId = decodeURIComponent(loginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session)
          return jsonResponse({ message: 'missing mock session' }, 404)
        const body = parseBody<UpdateOauthLoginSessionPayload>(init?.body, {})
        session.displayName = body.displayName?.trim() || undefined
        session.groupName = body.groupName?.trim() || undefined
        session.note = body.note?.trim() || undefined
        session.groupNote = body.groupNote?.trim() || undefined
        session.tagIds = Array.isArray(body.tagIds) ? body.tagIds : []
        session.isMother = body.isMother === true
        session.mailboxSessionId = body.mailboxSessionId?.trim() || undefined
        session.mailboxAddress = body.mailboxAddress?.trim() || undefined
        return jsonResponse(clone(session))
      }
      if (loginSessionMatch && method === 'GET') {
        const loginId = decodeURIComponent(loginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session)
          return jsonResponse({ message: 'missing mock session' }, 404)
        return jsonResponse(clone(session))
      }

      const completeLoginSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)\/complete$/,
      )
      if (completeLoginSessionMatch && method === 'POST') {
        const loginId = decodeURIComponent(completeLoginSessionMatch[1])
        const session = store.sessions[loginId]
        if (!session)
          return jsonResponse({ message: 'missing mock session' }, 404)
        const body = parseBody<CompleteOauthLoginSessionPayload>(init?.body, {
          callbackUrl: '',
        })
        const callbackUrl = body.callbackUrl.trim()
        if (
          !callbackUrl ||
          !session.state ||
          !callbackUrl.includes(session.state)
        ) {
          session.status = 'failed'
          session.error =
            'Mock callback URL does not contain the expected state token.'
          return jsonResponse({ message: session.error }, 400)
        }
        const nextId = session.accountId ?? store.nextId++
        const existing = store.details[nextId]
        const detail = createOauthAccount(nextId, {
          displayName:
            session.displayName ||
            existing?.displayName ||
            'Codex Pro - New login',
          groupName: session.groupName ?? existing?.groupName ?? 'default',
          isMother: session.isMother ?? existing?.isMother ?? false,
          note:
            session.note ??
            existing?.note ??
            'Freshly connected from Storybook OAuth mock.',
        })
        const normalizedGroupName = normalizeGroupName(detail.groupName)
        if (normalizedGroupName && session.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = session.groupNote.trim()
        }
        store.details[nextId] = detail
        const summary = toSummary(detail)
        store.accounts = [
          summary,
          ...store.accounts.filter((item) => item.id !== nextId),
        ]
        session.accountId = nextId
        session.status = 'completed'
        session.authUrl = null
        session.redirectUri = null
        session.error = null
        return jsonResponse(clone(detail))
      }

      if (
        path === '/api/pool/upstream-accounts/api-keys' &&
        method === 'POST'
      ) {
        const body = parseBody<CreateApiKeyAccountPayload>(init?.body, {
          displayName: 'New API key',
          apiKey: 'sk-storybook-key',
        })
        const nextId = store.nextId++
        const detail = createApiKeyAccount(nextId, {
          displayName: body.displayName,
          groupName: body.groupName ?? 'default',
          isMother: body.isMother === true,
          note: body.note ?? null,
          upstreamBaseUrl: body.upstreamBaseUrl ?? null,
          maskedApiKey: maskApiKey(body.apiKey),
          localLimits: {
            primaryLimit: body.localPrimaryLimit ?? 120,
            secondaryLimit: body.localSecondaryLimit ?? 500,
            limitUnit: body.localLimitUnit ?? 'requests',
          },
        })
        const synced = syncLocalWindows(detail)
        const normalizedGroupName = normalizeGroupName(synced.groupName)
        if (normalizedGroupName && body.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = body.groupNote.trim()
        }
        store.details[nextId] = synced
        store.accounts = [toSummary(synced), ...store.accounts]
        return jsonResponse(clone(synced), 201)
      }

      const reloginMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/oauth\/relogin$/,
      )
      if (reloginMatch && method === 'POST') {
        const accountId = Number(reloginMatch[1])
        const state = `state_relogin_${accountId}`
        const session: StoryStore['sessions'][string] = {
          loginId: `relogin_${accountId}_${Date.now()}`,
          status: 'pending',
          authUrl: `https://auth.openai.com/authorize?mock=1&accountId=${accountId}&state=${state}`,
          redirectUri: `http://localhost:432${String(accountId).slice(-1)}/oauth/callback`,
          expiresAt: storyFutureLoginExpiresAt,
          accountId,
          error: null,
          state,
        }
        store.sessions[session.loginId] = session
        return jsonResponse(clone(session), 201)
      }

      const syncMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/sync$/,
      )
      if (syncMatch && method === 'POST') {
        const accountId = Number(syncMatch[1])
        const detail = store.details[accountId]
        if (!detail)
          return jsonResponse({ message: 'missing mock account' }, 404)
        const updated = syncLocalWindows({
          ...detail,
          status: 'active',
          lastSyncedAt: now,
          lastSuccessfulSyncAt: now,
          lastError: null,
          lastErrorAt: null,
        })
        store.details[accountId] = updated
        store.accounts = store.accounts.map((item) =>
          item.id === accountId ? toSummary(updated) : item,
        )
        return jsonResponse(clone(updated))
      }

      const detailMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/)
      if (detailMatch && method === 'GET') {
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail)
          return jsonResponse({ message: 'missing mock account' }, 404)
        return jsonResponse(clone(detail))
      }

      if (path === '/api/invocations' && method === 'GET') {
        const requestedAccountId = Number(parsedUrl.searchParams.get('upstreamAccountId') || 0)
        const stickyKey = parsedUrl.searchParams.get('stickyKey')?.trim() || ''
        const pageSize = Math.max(1, Number(parsedUrl.searchParams.get('pageSize') || 20))
        const page = Math.max(1, Number(parsedUrl.searchParams.get('page') || 1))
        const allRecords = buildStickyInvocationRecords(
          requestedAccountId > 0 ? requestedAccountId : 101,
        )
        const filteredRecords = allRecords.filter((record) => (
          (requestedAccountId > 0 ? record.upstreamAccountId === requestedAccountId : true)
          && (stickyKey ? record.promptCacheKey === stickyKey : true)
        ))
        const start = (page - 1) * pageSize
        return jsonResponse({
          snapshotId: 1,
          total: filteredRecords.length,
          page,
          pageSize,
          records: filteredRecords.slice(start, start + pageSize),
        })
      }

      const stickyMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/sticky-keys$/,
      )
      if (stickyMatch && method === 'GET') {
        const accountId = Number(stickyMatch[1])
        return jsonResponse(buildStickyConversations(accountId, parsedUrl))
      }

      if (detailMatch && method === 'PATCH') {
        if (storyId?.endsWith('--completed-save-failure-feedback')) {
          return Promise.resolve(
            new Response('Storybook forced save failure.', {
              status: 500,
              headers: { 'Content-Type': 'text/plain' },
            }),
          )
        }
        const accountId = Number(detailMatch[1])
        const detail = store.details[accountId]
        if (!detail)
          return jsonResponse({ message: 'missing mock account' }, 404)
        const body = parseBody<UpdateUpstreamAccountPayload>(init?.body, {})
        const updated = syncLocalWindows({
          ...detail,
          displayName: body.displayName ?? detail.displayName,
          groupName: body.groupName ?? detail.groupName,
          isMother: body.isMother ?? detail.isMother,
          note: body.note ?? detail.note,
          upstreamBaseUrl:
            detail.kind === 'api_key_codex' &&
            Object.prototype.hasOwnProperty.call(body, 'upstreamBaseUrl')
              ? (body.upstreamBaseUrl ?? null)
              : detail.upstreamBaseUrl,
          enabled: body.enabled ?? detail.enabled,
          status:
            body.enabled === false
              ? 'disabled'
              : detail.status === 'disabled'
                ? 'active'
                : detail.status,
          maskedApiKey: body.apiKey
            ? maskApiKey(body.apiKey)
            : detail.maskedApiKey,
          localLimits:
            detail.kind === 'api_key_codex'
              ? {
                  primaryLimit:
                    body.localPrimaryLimit ??
                    detail.localLimits?.primaryLimit ??
                    120,
                  secondaryLimit:
                    body.localSecondaryLimit ??
                    detail.localLimits?.secondaryLimit ??
                    500,
                  limitUnit:
                    body.localLimitUnit ??
                    detail.localLimits?.limitUnit ??
                    'requests',
                }
              : detail.localLimits,
        })
        store.details[accountId] = updated
        store.accounts = store.accounts.map((item) =>
          item.id === accountId ? toSummary(updated) : item,
        )
        return jsonResponse(clone(updated))
      }

      const groupMatch = path.match(
        /^\/api\/pool\/upstream-account-groups\/(.+)$/,
      )
      if (groupMatch && method === 'PUT') {
        const groupName = decodeURIComponent(groupMatch[1])
        const body = parseBody<UpdateUpstreamAccountGroupPayload>(
          init?.body,
          {},
        )
        const normalized = normalizeGroupName(groupName)
        if (!normalized)
          return jsonResponse({ message: 'missing mock group' }, 404)
        const note = body.note?.trim() ?? ''
        const boundProxyKeys = Array.isArray(body.boundProxyKeys)
          ? Array.from(
              new Set(
                body.boundProxyKeys
                  .map((value) => value.trim())
                  .filter(Boolean),
              ),
            )
          : []
        if (note) store.groupNotes[normalized] = note
        else delete store.groupNotes[normalized]
        if (boundProxyKeys.length > 0) {
          store.groupBoundProxyKeys[normalized] = boundProxyKeys
        } else {
          delete store.groupBoundProxyKeys[normalized]
        }
        return jsonResponse({
          groupName: normalized,
          note: note || null,
          boundProxyKeys,
        })
      }

      if (detailMatch && method === 'DELETE') {
        const accountId = Number(detailMatch[1])
        if (storyId?.endsWith('--delete-failure')) {
          return Promise.resolve(
            new Response(
              'error returned from database: (code: 5) database is locked',
              {
                status: 500,
                headers: { 'Content-Type': 'text/plain' },
              },
            ),
          )
        }
        delete store.details[accountId]
        store.accounts = store.accounts.filter((item) => item.id !== accountId)
        return noContent()
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init)
    }

    window.fetch = mockedFetch
    window.EventSource = class extends MockStoryBulkSyncEventSource {
      constructor(url: string | URL) {
        super(storeRef, url)
      }
    } as unknown as typeof window.EventSource
  }

  useEffect(() => {
    return () => {
      if (typeof window !== 'undefined' && originalFetchRef.current) {
        window.fetch = originalFetchRef.current
        originalFetchRef.current = null
      }
      if (typeof window !== 'undefined' && originalEventSourceRef.current) {
        window.EventSource = originalEventSourceRef.current
        originalEventSourceRef.current = null
      }
    }
  }, [])

  return <>{children}</>
}

export function AccountPoolStoryRouter({
  initialEntry,
}: {
  initialEntry: StoryInitialEntry
}) {
  const { themeMode } = useTheme()
  const isDark = themeMode === 'dark'
  return (
    <div
      className="min-h-screen bg-base-200 px-6 py-6 text-base-content"
      style={{
        backgroundImage: isDark
          ? 'radial-gradient(circle at 10% -10%, rgba(56,189,248,0.18), transparent 36%), radial-gradient(circle at 88% 0%, rgba(45,212,191,0.16), transparent 34%), linear-gradient(180deg, #081428 0%, #10213a 62%)'
          : 'radial-gradient(circle at 10% -10%, rgba(14,165,233,0.10), transparent 34%), radial-gradient(circle at 88% 0%, rgba(16,185,129,0.10), transparent 30%), linear-gradient(180deg, #f7fbff 0%, #e8f1fb 58%, #e1ecf8 100%)',
      }}
    >
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/account-pool" element={<AccountPoolLayout />}>
            <Route
              path="upstream-accounts"
              element={<UpstreamAccountsPage />}
            />
            <Route
              path="upstream-accounts/new"
              element={<UpstreamAccountCreatePage />}
            />
          </Route>
        </Routes>
      </MemoryRouter>
    </div>
  )
}
