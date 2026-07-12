import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";

const UPSTREAM_ACCOUNT_ID_PARAM = "upstreamAccountId";
const UPSTREAM_ACCOUNT_TAB_PARAM = "upstreamAccountTab";
const PROMPT_CACHE_CONVERSATION_KEY_PARAM = "promptCacheConversationKey";
const PROMPT_CACHE_CONVERSATION_TAB_PARAM = "promptCacheConversationTab";

export type UpstreamAccountDetailRouteTab =
  "overview" | "records" | "edit" | "routing" | "healthEvents";

function parseUpstreamAccountTab(
  raw: string | null,
): UpstreamAccountDetailRouteTab {
  if (raw === "records") return "records";
  if (raw === "edit") return "edit";
  if (raw === "routing") return "routing";
  if (raw === "healthEvents") return "healthEvents";
  return "overview";
}

function parseUpstreamAccountId(raw: string | null) {
  if (!raw) return null;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) return null;
  const accountId = Math.trunc(parsed);
  return accountId > 0 ? accountId : null;
}

export function useUpstreamAccountDetailRoute() {
  const [searchParams, setSearchParams] = useSearchParams();
  const upstreamAccountId = useMemo(
    () => parseUpstreamAccountId(searchParams.get(UPSTREAM_ACCOUNT_ID_PARAM)),
    [searchParams],
  );
  const upstreamAccountTab = useMemo(
    () => parseUpstreamAccountTab(searchParams.get(UPSTREAM_ACCOUNT_TAB_PARAM)),
    [searchParams],
  );

  const openUpstreamAccount = useCallback(
    (
      accountId: number,
      options?: {
        replace?: boolean;
        tab?: UpstreamAccountDetailRouteTab;
        clearPromptCacheConversation?: boolean;
      },
    ) => {
      setSearchParams((currentSearchParams) => {
        const next = new URLSearchParams(currentSearchParams);
        if (options?.clearPromptCacheConversation) {
          next.delete(PROMPT_CACHE_CONVERSATION_KEY_PARAM);
          next.delete(PROMPT_CACHE_CONVERSATION_TAB_PARAM);
        }
        next.set(UPSTREAM_ACCOUNT_ID_PARAM, String(Math.trunc(accountId)));
        const tab = options?.tab ?? "overview";
        if (tab !== "overview") {
          next.set(UPSTREAM_ACCOUNT_TAB_PARAM, tab);
        } else {
          next.delete(UPSTREAM_ACCOUNT_TAB_PARAM);
        }
        return next;
      }, { replace: options?.replace ?? false });
    },
    [setSearchParams],
  );

  const closeUpstreamAccount = useCallback(
    (options?: { replace?: boolean }) => {
      if (
        !searchParams.has(UPSTREAM_ACCOUNT_ID_PARAM) &&
        !searchParams.has(UPSTREAM_ACCOUNT_TAB_PARAM)
      ) {
        return;
      }
      setSearchParams((currentSearchParams) => {
        const next = new URLSearchParams(currentSearchParams);
        next.delete(UPSTREAM_ACCOUNT_ID_PARAM);
        next.delete(UPSTREAM_ACCOUNT_TAB_PARAM);
        return next;
      }, { replace: options?.replace ?? false });
    },
    [searchParams, setSearchParams],
  );

  return {
    upstreamAccountId,
    upstreamAccountTab,
    openUpstreamAccount,
    closeUpstreamAccount,
  };
}
