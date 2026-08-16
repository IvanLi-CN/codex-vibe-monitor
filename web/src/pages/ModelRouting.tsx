import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ModelRoutingLivePanel } from "../features/live/ModelRoutingLivePanel";
import { useCompactViewport } from "../hooks/useCompactViewport";
import { useModelRoutingLive } from "../hooks/useModelRoutingLive";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

export default function ModelRoutingPage() {
  const navigate = useNavigate();
  const isCompactViewport = useCompactViewport();
  const {
    upstreamAccountId,
    upstreamAccountTab,
    upstreamAccountModel,
    openUpstreamAccount,
    closeUpstreamAccount,
  } = useUpstreamAccountDetailRoute();
  const [routingWindow, setRoutingWindow] = useState<"15m" | "1h" | "6h" | "24h">("24h");
  const [routingModel, setRoutingModel] = useState("");
  const [routingState, setRoutingState] = useState("");
  const {
    data: modelRouting,
    isLoading: modelRoutingLoading,
    error: modelRoutingError,
    refresh: refreshModelRouting,
  } = useModelRoutingLive(
    {
      window: routingWindow,
      model: routingModel || undefined,
      state: routingState || undefined,
      limit: 100,
    },
    true,
  );

  if (isCompactViewport && upstreamAccountId != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <SharedUpstreamAccountDetailDrawer
          open
          presentation="page"
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          initialExpandedModel={upstreamAccountModel}
          onClose={closeUpstreamAccount}
        />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <ModelRoutingLivePanel
        data={modelRouting}
        isLoading={modelRoutingLoading}
        error={modelRoutingError}
        window={routingWindow}
        model={routingModel}
        state={routingState}
        onWindowChange={setRoutingWindow}
        onModelChange={setRoutingModel}
        onStateChange={setRoutingState}
        onOpenAccount={(accountId, selectedModel) =>
          openUpstreamAccount(accountId, { tab: "healthEvents", model: selectedModel })
        }
        onOpenInvocation={(invokeId) =>
          navigate(`/records?invokeId=${encodeURIComponent(invokeId)}`)
        }
        onRefresh={refreshModelRouting}
      />
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          initialTab={upstreamAccountTab}
          initialExpandedModel={upstreamAccountModel}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  );
}
