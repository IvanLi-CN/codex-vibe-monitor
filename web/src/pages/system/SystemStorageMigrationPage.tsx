import type { JSX } from "react";
import { useCallback, useEffect, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { AppIcon } from "../../features/shared/AppIcon";
import { ListBodyState } from "../../features/shared/ListBodyState";
import StorageMigrationStatusCard from "../../features/system/StorageMigrationStatusCard";
import { useTranslation } from "../../i18n";
import {
  createStorageMigrationEventSource,
  fetchStorageMigrationStatus,
  normalizeStorageMigrationResponse,
  type StorageMigrationControlAction,
  type StorageMigrationResponse,
  updateStorageMigrationControl,
} from "../../lib/api";

const REFRESH_INTERVAL_MS = 15_000;

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export default function SystemStorageMigrationPage(): JSX.Element {
  const { t } = useTranslation();
  const [status, setStatus] = useState<StorageMigrationResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isActionPending, setIsActionPending] = useState(false);
  const [streamConnected, setStreamConnected] = useState(false);
  const [adminToken, setAdminToken] = useState("");

  const refresh = useCallback(async (signal?: AbortSignal) => {
    const next = await fetchStorageMigrationStatus(signal);
    setStatus((current) => (current && next.revision < current.revision ? current : next));
    setError(null);
    setIsLoading(false);
  }, []);

  useEffect(() => {
    let active = true;
    const controller = new AbortController();
    const load = async () => {
      try {
        await refresh(controller.signal);
      } catch (cause) {
        if (!active || controller.signal.aborted) return;
        setError(errorText(cause));
        setIsLoading(false);
      }
    };
    void load();

    const interval = window.setInterval(() => {
      void load();
    }, REFRESH_INTERVAL_MS);

    let source: EventSource | null = null;
    if (typeof window !== "undefined" && "EventSource" in window) {
      try {
        source = createStorageMigrationEventSource();
        source.onopen = () => {
          if (active) setStreamConnected(true);
        };
        const applyEvent = (event: Event) => {
          if (!active) return;
          try {
            const next = normalizeStorageMigrationResponse(
              JSON.parse((event as MessageEvent<string>).data) as unknown,
            );
            setStatus((current) => (current && next.revision < current.revision ? current : next));
            setError(null);
            setIsLoading(false);
          } catch (cause) {
            setError(errorText(cause));
          }
        };
        source.addEventListener("snapshot", applyEvent);
        source.addEventListener("status", applyEvent);
        source.addEventListener("progress", applyEvent);
        source.onerror = () => {
          if (active) setStreamConnected(false);
        };
      } catch (cause) {
        setError(errorText(cause));
      }
    }

    return () => {
      active = false;
      controller.abort();
      window.clearInterval(interval);
      source?.close();
    };
  }, [refresh]);

  const handleAction = async (action: StorageMigrationControlAction) => {
    setIsActionPending(true);
    try {
      const next = await updateStorageMigrationControl(action === "pause", { adminToken });
      setStatus((current) => (current && next.revision < current.revision ? current : next));
      setError(null);
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setIsActionPending(false);
    }
  };

  if (isLoading && !status) {
    return (
      <ListBodyState
        variant="loading"
        title={t("system.storageMigration.loading")}
        testId="storage-migration-loading"
      />
    );
  }

  if (error && !status) {
    return (
      <ListBodyState
        variant="error"
        title={t("system.storageMigration.loadError", { error })}
        retryLabel={t("system.storageMigration.retry")}
        onRetry={() => {
          setIsLoading(true);
          void refresh().catch((cause) => {
            setError(errorText(cause));
            setIsLoading(false);
          });
        }}
        testId="storage-migration-error"
      />
    );
  }

  return (
    <div className="grid gap-4" data-testid="system-storage-migration-page">
      {!streamConnected ? (
        <Alert variant="warning">
          <AppIcon name="refresh" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          {t("system.storageMigration.streamFallback")}
        </Alert>
      ) : null}
      {status ? (
        <StorageMigrationStatusCard
          status={status}
          error={error}
          isActionPending={isActionPending}
          adminToken={adminToken}
          onAdminTokenChange={setAdminToken}
          onAction={(action) => void handleAction(action)}
        />
      ) : null}
    </div>
  );
}
