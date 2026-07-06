import { useCallback, useMemo } from "react";
import { useSearchParams } from "react-router-dom";

const UPSTREAM_ACCOUNT_ID_PARAM = "upstreamAccountId";
const UPSTREAM_ACCOUNT_TAB_PARAM = "upstreamAccountTab";

export type UpstreamAccountDetailRouteTab = "overview" | "routing";

function parseUpstreamAccountTab(
  raw: string | null,
): UpstreamAccountDetailRouteTab {
  if (raw === "routing") return "routing";
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
      },
    ) => {
      const next = new URLSearchParams(searchParams);
      next.set(UPSTREAM_ACCOUNT_ID_PARAM, String(Math.trunc(accountId)));
      if ((options?.tab ?? "overview") === "routing") {
        next.set(UPSTREAM_ACCOUNT_TAB_PARAM, "routing");
      } else {
        next.delete(UPSTREAM_ACCOUNT_TAB_PARAM);
      }
      setSearchParams(next, { replace: options?.replace ?? false });
    },
    [searchParams, setSearchParams],
  );

  const closeUpstreamAccount = useCallback(
    (options?: { replace?: boolean }) => {
      if (
        !searchParams.has(UPSTREAM_ACCOUNT_ID_PARAM) &&
        !searchParams.has(UPSTREAM_ACCOUNT_TAB_PARAM)
      ) {
        return;
      }
      const next = new URLSearchParams(searchParams);
      next.delete(UPSTREAM_ACCOUNT_ID_PARAM);
      next.delete(UPSTREAM_ACCOUNT_TAB_PARAM);
      setSearchParams(next, { replace: options?.replace ?? false });
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
