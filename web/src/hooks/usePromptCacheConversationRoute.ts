import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import type { BlockedBindingConstraintSource } from "../lib/api";

const PROMPT_CACHE_CONVERSATION_KEY_PARAM = "promptCacheConversationKey";
const PROMPT_CACHE_CONVERSATION_TAB_PARAM = "promptCacheConversationTab";
const UPSTREAM_ACCOUNT_ID_PARAM = "upstreamAccountId";
const UPSTREAM_ACCOUNT_TAB_PARAM = "upstreamAccountTab";
const BLOCKED_BINDING_UPSTREAM_ACCOUNT_ID_PARAM = "blockedBindingUpstreamAccountId";
const BLOCKED_BINDING_CONSTRAINT_SOURCE_PARAM = "blockedBindingConstraintSource";

export type PromptCacheConversationRouteTab = "overview" | "calls" | "settings" | "operations";

function parsePromptCacheConversationTab(raw: string | null): PromptCacheConversationRouteTab {
  if (raw === "calls") return "calls";
  if (raw === "settings") return "settings";
  if (raw === "operations") return "operations";
  return "overview";
}

function parsePromptCacheConversationKey(raw: string | null) {
  const normalized = raw?.trim() ?? "";
  return normalized.length > 0 ? normalized : null;
}

function parseBlockedBindingUpstreamAccountId(raw: string | null) {
  if (!raw) return null;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) return null;
  return Math.trunc(parsed);
}

function parseBlockedBindingConstraintSource(
  raw: string | null,
): BlockedBindingConstraintSource | null {
  const normalized = raw?.trim() ?? "";
  return normalized.length > 0 ? normalized : null;
}

export function usePromptCacheConversationRoute() {
  const [searchParams, setSearchParams] = useSearchParams();
  const promptCacheConversationKey = useMemo(
    () => parsePromptCacheConversationKey(searchParams.get(PROMPT_CACHE_CONVERSATION_KEY_PARAM)),
    [searchParams],
  );
  const promptCacheConversationTab = useMemo(
    () => parsePromptCacheConversationTab(searchParams.get(PROMPT_CACHE_CONVERSATION_TAB_PARAM)),
    [searchParams],
  );
  const blockedBindingUpstreamAccountId = useMemo(
    () =>
      parseBlockedBindingUpstreamAccountId(
        searchParams.get(BLOCKED_BINDING_UPSTREAM_ACCOUNT_ID_PARAM),
      ),
    [searchParams],
  );
  const blockedBindingConstraintSource = useMemo(
    () =>
      parseBlockedBindingConstraintSource(
        searchParams.get(BLOCKED_BINDING_CONSTRAINT_SOURCE_PARAM),
      ),
    [searchParams],
  );
  const blockedBindingFilter = useMemo(() => {
    if (blockedBindingUpstreamAccountId == null && blockedBindingConstraintSource == null) {
      return null;
    }
    return {
      upstreamAccountId: blockedBindingUpstreamAccountId,
      constraintSource: blockedBindingConstraintSource,
    };
  }, [blockedBindingConstraintSource, blockedBindingUpstreamAccountId]);

  const openPromptCacheConversation = useCallback(
    (
      conversationKey: string,
      options?: {
        replace?: boolean;
        tab?: PromptCacheConversationRouteTab;
        clearUpstreamAccount?: boolean;
      },
    ) => {
      const normalizedKey = conversationKey.trim();
      if (!normalizedKey) return;

      setSearchParams(
        (currentSearchParams) => {
          const next = new URLSearchParams(currentSearchParams);
          if (options?.clearUpstreamAccount) {
            next.delete(UPSTREAM_ACCOUNT_ID_PARAM);
            next.delete(UPSTREAM_ACCOUNT_TAB_PARAM);
          }
          next.set(PROMPT_CACHE_CONVERSATION_KEY_PARAM, normalizedKey);
          const tab = options?.tab ?? "overview";
          if (tab !== "overview") {
            next.set(PROMPT_CACHE_CONVERSATION_TAB_PARAM, tab);
          } else {
            next.delete(PROMPT_CACHE_CONVERSATION_TAB_PARAM);
          }
          return next;
        },
        { replace: options?.replace ?? false },
      );
    },
    [setSearchParams],
  );

  const closePromptCacheConversation = useCallback(
    (options?: { replace?: boolean }) => {
      if (
        !searchParams.has(PROMPT_CACHE_CONVERSATION_KEY_PARAM) &&
        !searchParams.has(PROMPT_CACHE_CONVERSATION_TAB_PARAM)
      ) {
        return;
      }

      setSearchParams(
        (currentSearchParams) => {
          const next = new URLSearchParams(currentSearchParams);
          next.delete(PROMPT_CACHE_CONVERSATION_KEY_PARAM);
          next.delete(PROMPT_CACHE_CONVERSATION_TAB_PARAM);
          return next;
        },
        { replace: options?.replace ?? false },
      );
    },
    [searchParams, setSearchParams],
  );

  const openBlockedBindingConversations = useCallback(
    (
      filter: {
        upstreamAccountId?: number | null;
        constraintSource?: BlockedBindingConstraintSource | null;
      },
      options?: {
        replace?: boolean;
        clearPromptCacheConversation?: boolean;
        clearUpstreamAccount?: boolean;
      },
    ) => {
      setSearchParams(
        (currentSearchParams) => {
          const next = new URLSearchParams(currentSearchParams);
          if (options?.clearPromptCacheConversation) {
            next.delete(PROMPT_CACHE_CONVERSATION_KEY_PARAM);
            next.delete(PROMPT_CACHE_CONVERSATION_TAB_PARAM);
          }
          if (options?.clearUpstreamAccount) {
            next.delete(UPSTREAM_ACCOUNT_ID_PARAM);
            next.delete(UPSTREAM_ACCOUNT_TAB_PARAM);
          }
          const upstreamAccountId = filter.upstreamAccountId;
          if (upstreamAccountId != null && Number.isFinite(upstreamAccountId)) {
            next.set(
              BLOCKED_BINDING_UPSTREAM_ACCOUNT_ID_PARAM,
              String(Math.trunc(upstreamAccountId)),
            );
          } else {
            next.delete(BLOCKED_BINDING_UPSTREAM_ACCOUNT_ID_PARAM);
          }
          const constraintSource = filter.constraintSource?.trim();
          if (constraintSource) {
            next.set(BLOCKED_BINDING_CONSTRAINT_SOURCE_PARAM, constraintSource);
          } else {
            next.delete(BLOCKED_BINDING_CONSTRAINT_SOURCE_PARAM);
          }
          return next;
        },
        { replace: options?.replace ?? false },
      );
    },
    [setSearchParams],
  );

  const clearBlockedBindingFilter = useCallback(
    (options?: { replace?: boolean }) => {
      if (
        !searchParams.has(BLOCKED_BINDING_UPSTREAM_ACCOUNT_ID_PARAM) &&
        !searchParams.has(BLOCKED_BINDING_CONSTRAINT_SOURCE_PARAM)
      ) {
        return;
      }
      setSearchParams(
        (currentSearchParams) => {
          const next = new URLSearchParams(currentSearchParams);
          next.delete(BLOCKED_BINDING_UPSTREAM_ACCOUNT_ID_PARAM);
          next.delete(BLOCKED_BINDING_CONSTRAINT_SOURCE_PARAM);
          return next;
        },
        { replace: options?.replace ?? false },
      );
    },
    [searchParams, setSearchParams],
  );

  return {
    promptCacheConversationKey,
    promptCacheConversationTab,
    blockedBindingFilter,
    openPromptCacheConversation,
    closePromptCacheConversation,
    openBlockedBindingConversations,
    clearBlockedBindingFilter,
  };
}
