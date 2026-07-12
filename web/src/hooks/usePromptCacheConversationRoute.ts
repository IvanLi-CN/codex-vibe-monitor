import { useCallback, useMemo } from 'react'
import { useSearchParams } from 'react-router-dom'

const PROMPT_CACHE_CONVERSATION_KEY_PARAM = 'promptCacheConversationKey'
const PROMPT_CACHE_CONVERSATION_TAB_PARAM = 'promptCacheConversationTab'
const UPSTREAM_ACCOUNT_ID_PARAM = 'upstreamAccountId'
const UPSTREAM_ACCOUNT_TAB_PARAM = 'upstreamAccountTab'

export type PromptCacheConversationRouteTab = 'overview' | 'calls' | 'settings'

function parsePromptCacheConversationTab(raw: string | null): PromptCacheConversationRouteTab {
  if (raw === 'calls') return 'calls'
  if (raw === 'settings') return 'settings'
  return 'overview'
}

function parsePromptCacheConversationKey(raw: string | null) {
  const normalized = raw?.trim() ?? ''
  return normalized.length > 0 ? normalized : null
}

export function usePromptCacheConversationRoute() {
  const [searchParams, setSearchParams] = useSearchParams()
  const promptCacheConversationKey = useMemo(
    () => parsePromptCacheConversationKey(searchParams.get(PROMPT_CACHE_CONVERSATION_KEY_PARAM)),
    [searchParams],
  )
  const promptCacheConversationTab = useMemo(
    () => parsePromptCacheConversationTab(searchParams.get(PROMPT_CACHE_CONVERSATION_TAB_PARAM)),
    [searchParams],
  )

  const openPromptCacheConversation = useCallback(
    (
      conversationKey: string,
      options?: {
        replace?: boolean
        tab?: PromptCacheConversationRouteTab
        clearUpstreamAccount?: boolean
      },
    ) => {
      const normalizedKey = conversationKey.trim()
      if (!normalizedKey) return

      setSearchParams((currentSearchParams) => {
        const next = new URLSearchParams(currentSearchParams)
        if (options?.clearUpstreamAccount) {
          next.delete(UPSTREAM_ACCOUNT_ID_PARAM)
          next.delete(UPSTREAM_ACCOUNT_TAB_PARAM)
        }
        next.set(PROMPT_CACHE_CONVERSATION_KEY_PARAM, normalizedKey)
        const tab = options?.tab ?? 'overview'
        if (tab !== 'overview') {
          next.set(PROMPT_CACHE_CONVERSATION_TAB_PARAM, tab)
        } else {
          next.delete(PROMPT_CACHE_CONVERSATION_TAB_PARAM)
        }
        return next
      }, { replace: options?.replace ?? false })
    },
    [setSearchParams],
  )

  const closePromptCacheConversation = useCallback(
    (options?: { replace?: boolean }) => {
      if (
        !searchParams.has(PROMPT_CACHE_CONVERSATION_KEY_PARAM) &&
        !searchParams.has(PROMPT_CACHE_CONVERSATION_TAB_PARAM)
      ) {
        return
      }

      setSearchParams((currentSearchParams) => {
        const next = new URLSearchParams(currentSearchParams)
        next.delete(PROMPT_CACHE_CONVERSATION_KEY_PARAM)
        next.delete(PROMPT_CACHE_CONVERSATION_TAB_PARAM)
        return next
      }, { replace: options?.replace ?? false })
    },
    [searchParams, setSearchParams],
  )

  return {
    promptCacheConversationKey,
    promptCacheConversationTab,
    openPromptCacheConversation,
    closePromptCacheConversation,
  }
}
