import { createContext, type ReactNode, useContext } from "react";
import { Outlet } from "react-router-dom";
import { Button } from "../../components/ui/button";
import { useAppVersion } from "../../hooks/useAppVersion";
import usePwaRuntime from "../../hooks/usePwaRuntime";
import useSseDiagnostics from "../../hooks/useSseDiagnostics";
import useSseStatus from "../../hooks/useSseStatus";
import useUpdateAvailable from "../../hooks/useUpdateAvailable";
import { useTranslation } from "../../i18n";
import {
  requestImmediateReconnect,
  type SseDiagnostics,
  type SseReconnectReason,
  type SseStatus,
  type SseTerminalOutcome,
} from "../../lib/sse";
import { AppIcon } from "../shared/AppIcon";
import {
  AppFooter,
  AppHeader,
  AppInstallDialog,
  AppPwaOfflineBanner,
  AppUpdateBanners,
  useHeaderBrandMarkState,
  usePwaInstallDialog,
} from "./AppLayoutSections";

const OFFLINE_NOTICE_THRESHOLD_MS = 2 * 60 * 1000;
export const HEADER_BRAND_ACTIVITY_HOLD_MS = 3200;

export type SseOfflineBannerStoryState = {
  status: SseStatus;
  diagnostics: SseDiagnostics;
};

const SseOfflineBannerStoryStateContext = createContext<SseOfflineBannerStoryState | null>(null);

export function SseOfflineBannerStoryStateProvider({
  children,
  state,
}: {
  children: ReactNode;
  state?: SseOfflineBannerStoryState;
}) {
  return (
    <SseOfflineBannerStoryStateContext.Provider value={state ?? null}>
      {children}
    </SseOfflineBannerStoryStateContext.Provider>
  );
}

function formatDiagnosticsAgeLabel(
  timestamp: number | null,
  now: number,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (timestamp == null) {
    return t("app.sse.banner.diagAgeNever");
  }
  const totalSeconds = Math.max(Math.floor((now - timestamp) / 1000), 0);
  if (totalSeconds < 60) {
    return t("app.sse.banner.diagAgeSeconds", { seconds: totalSeconds });
  }
  return t("app.sse.banner.diagAgeMinutesSeconds", {
    minutes: Math.floor(totalSeconds / 60),
    seconds: totalSeconds % 60,
  });
}

function reasonLabelKey(reason: SseReconnectReason | null) {
  switch (reason) {
    case "initial":
      return "app.sse.reason.initial";
    case "topic-change":
      return "app.sse.reason.topicChange";
    case "topic-refresh":
      return "app.sse.reason.topicRefresh";
    case "manual":
      return "app.sse.reason.manual";
    case "eventsource-error":
      return "app.sse.reason.eventsourceError";
    case "watchdog-closed":
      return "app.sse.reason.watchdogClosed";
    case "watchdog-timeout":
      return "app.sse.reason.watchdogTimeout";
    case "visibility-visible":
      return "app.sse.reason.visibilityVisible";
    default:
      return "app.sse.banner.diagUnknown";
  }
}

function outcomeLabelKey(outcome: SseTerminalOutcome | null) {
  switch (outcome) {
    case "idle":
      return "app.sse.outcome.idle";
    case "open":
      return "app.sse.outcome.open";
    case "topic-change":
      return "app.sse.outcome.topicChange";
    case "eventsource-error":
      return "app.sse.outcome.eventsourceError";
    case "watchdog-closed":
      return "app.sse.outcome.watchdogClosed";
    case "watchdog-timeout":
      return "app.sse.outcome.watchdogTimeout";
    case "disabled":
      return "app.sse.outcome.disabled";
    case "unsupported":
      return "app.sse.outcome.unsupported";
    case "cleanup":
      return "app.sse.outcome.cleanup";
    default:
      return "app.sse.banner.diagUnknown";
  }
}

function SseOfflineBanner({ topClassName }: { topClassName: string }) {
  const { t } = useTranslation();
  const storyState = useContext(SseOfflineBannerStoryStateContext);
  const currentSseStatus = useSseStatus();
  const currentSseDiagnostics = useSseDiagnostics();
  const sseStatus = storyState?.status ?? currentSseStatus;
  const sseDiagnostics = storyState?.diagnostics ?? currentSseDiagnostics;
  const diagnosticsNow = Date.now();

  const isOffline = sseStatus.phase !== "connected" && sseStatus.phase !== "idle";
  const showOfflineBanner = isOffline && sseStatus.downtimeMs >= OFFLINE_NOTICE_THRESHOLD_MS;
  const downtimeSeconds = Math.max(Math.floor(sseStatus.downtimeMs / 1000), 0);
  const downtimeMinutesPart = Math.floor(downtimeSeconds / 60);
  const downtimeSecondsPart = downtimeSeconds % 60;
  const nextRetrySeconds =
    sseStatus.nextRetryAt != null
      ? Math.max(Math.ceil((sseStatus.nextRetryAt - Date.now()) / 1000), 0)
      : null;
  const durationChipLabel = t("app.sse.banner.durationChip", {
    minutes: downtimeMinutesPart,
    seconds: downtimeSecondsPart.toString().padStart(2, "0"),
  });
  const statusLine = sseStatus.autoReconnect
    ? nextRetrySeconds != null && nextRetrySeconds > 0
      ? t("app.sse.banner.retryIn", { seconds: nextRetrySeconds })
      : t("app.sse.banner.retryingNow")
    : t("app.sse.banner.autoDisabled");
  const diagnosticsLine = t("app.sse.banner.diagnostics", {
    attempt: sseDiagnostics.attempt ?? "-",
    reason: t(reasonLabelKey(sseDiagnostics.reason)),
    topics: sseDiagnostics.activeTopics.length,
    resume: sseDiagnostics.resumeTopics.length,
    fresh: sseDiagnostics.forcedSnapshotTopics.length,
    lastMessageAge: formatDiagnosticsAgeLabel(sseDiagnostics.lastMessageAt, diagnosticsNow, t),
    outcome: t(outcomeLabelKey(sseDiagnostics.lastTerminalOutcome)),
  });

  if (!showOfflineBanner) {
    return null;
  }

  return (
    <div className={`fixed left-1/2 z-[60] w-full max-w-3xl -translate-x-1/2 px-4 ${topClassName}`}>
      <div
        className="flex w-full gap-3 rounded-xl border border-warning/60 bg-warning/90 p-4 text-warning-content shadow-lg"
        role="status"
        aria-live="assertive"
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <AppIcon name="alert-circle" className="h-6 w-6 flex-shrink-0" aria-hidden />
          <div className="min-w-0 flex-1 space-y-1">
            <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:gap-4">
              <div className="min-w-0 flex-1 space-y-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-semibold">{t("app.sse.banner.title")}</span>
                  <span
                    className="text-xs font-mono tabular-nums text-warning-content/80"
                    data-testid="app-sse-downtime"
                  >
                    {durationChipLabel}
                  </span>
                </div>
                <p className="min-w-0 text-sm text-warning-content/90">
                  {t("app.sse.banner.description")} · {statusLine}
                </p>
              </div>
              <Button
                type="button"
                size="sm"
                className="hidden w-auto flex-shrink-0 px-5 sm:mt-1 sm:inline-flex"
                onClick={requestImmediateReconnect}
              >
                {t("app.sse.banner.reconnectButton")}
              </Button>
            </div>
            <p
              className="text-xs font-mono text-warning-content/80"
              data-testid="app-sse-diagnostics"
            >
              {diagnosticsLine}
            </p>
            <div className="flex justify-end sm:hidden" data-testid="app-sse-reconnect-mobile-row">
              <Button
                type="button"
                size="sm"
                className="w-auto px-5"
                onClick={requestImmediateReconnect}
              >
                {t("app.sse.banner.reconnectButton")}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export function AppLayout() {
  const { versionInfo, isLoading: backendLoading } = useAppVersion();
  const update = useUpdateAvailable();
  const pwaRuntime = usePwaRuntime();
  const sseStatus = useSseStatus();
  const headerBrandMarkState = useHeaderBrandMarkState(sseStatus.phase);
  const installDialog = usePwaInstallDialog(pwaRuntime);
  const showVersionUpdateBanner = pwaRuntime.update.visible || update.visible;
  const stackedStatusBannerTopClass = showVersionUpdateBanner ? "top-[146px]" : "top-[78px]";

  return (
    <div className="app-shell min-h-screen flex flex-col text-base-content">
      <AppInstallDialog state={installDialog} pwaRuntime={pwaRuntime} />
      <AppHeader
        brandMarkState={headerBrandMarkState}
        installDialogMode={installDialog.mode}
        onOpenInstallDialog={installDialog.open}
      />
      <AppPwaOfflineBanner
        isOffline={pwaRuntime.isOffline}
        shellReady={pwaRuntime.shellReady}
        topClassName={stackedStatusBannerTopClass}
      />
      <SseOfflineBanner topClassName={stackedStatusBannerTopClass} />
      <AppUpdateBanners
        pwaRuntime={pwaRuntime}
        update={update}
        backendVersion={versionInfo?.backend}
      />
      <main
        className="app-shell-boundary flex-1 min-h-0 px-3 py-5 pb-8 sm:px-4 sm:py-6"
        data-testid="app-main"
      >
        <Outlet />
      </main>
      <AppFooter backendVersion={versionInfo?.backend} isBackendLoading={backendLoading} />
    </div>
  );
}
