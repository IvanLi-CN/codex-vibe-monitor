import { useNavigate } from "react-router-dom";
import { useCompactViewport } from "../hooks/useCompactViewport";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";
import { LivePageSections } from "./LivePageSections";
import { useLivePageData } from "./useLivePageData";
import {
  DEFAULT_PROMPT_CACHE_SELECTION,
  PROMPT_CACHE_SELECTION_LOOKUP,
  useLivePageState,
} from "./useLivePageState";

export default function LivePage() {
  const navigate = useNavigate();
  const isCompactViewport = useCompactViewport();
  const route = useUpstreamAccountDetailRoute();
  const state = useLivePageState();
  const conversationSelection =
    PROMPT_CACHE_SELECTION_LOOKUP.get(state.conversationSelectionValue) ??
    DEFAULT_PROMPT_CACHE_SELECTION;
  const data = useLivePageData({
    activeTab: state.activeTab,
    limit: state.limit,
    summaryWindow: state.summaryWindow,
    conversationSelection,
    routingWindow: state.routingWindow,
  });

  if (isCompactViewport && route.upstreamAccountId != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <SharedUpstreamAccountDetailDrawer
          open
          presentation="page"
          accountId={route.upstreamAccountId}
          initialTab={route.upstreamAccountTab}
          initialExpandedModel={route.upstreamAccountModel}
          onClose={route.closeUpstreamAccount}
        />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <LivePageSections
        state={state}
        data={data}
        upstreamAccountId={route.upstreamAccountId}
        upstreamAccountTab={route.upstreamAccountTab}
        upstreamAccountModel={route.upstreamAccountModel}
        onOpenUpstreamAccount={(accountId) => route.openUpstreamAccount(accountId)}
        onCloseUpstreamAccount={route.closeUpstreamAccount}
        onOpenRoutingAccount={(accountId, model) =>
          route.openUpstreamAccount(accountId, { tab: "healthEvents", model })
        }
        onOpenInvocation={(invokeId) =>
          navigate(`/records?invokeId=${encodeURIComponent(invokeId)}`)
        }
      />
    </div>
  );
}
