import { type ComponentProps, useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { useTranslation } from "../../i18n";
import { fetchSystemTaskRuns, type SystemTaskRun } from "../../lib/api";
import { SystemTaskFilters, SystemTaskList, SystemTaskPagination } from "./SystemTaskSections";

function toIsoStringOrUndefined(value: string, upperBound = false): string | undefined {
  const normalized = value.trim();
  if (!normalized) return undefined;
  const parsed = new Date(normalized);
  if (Number.isNaN(parsed.getTime())) return undefined;
  if (upperBound && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(normalized)) {
    parsed.setSeconds(59, 999);
  }
  return parsed.toISOString();
}

function SystemTaskFilterPanel({
  filteredCount,
  onStartedAtFromChange,
  onStartedAtToChange,
  onStatusChange,
  onTaskKindChange,
  startedAtFrom,
  startedAtTo,
  status,
  taskKind,
  t,
}: Omit<ComponentProps<typeof SystemTaskFilters>, "t"> & {
  t: ReturnType<typeof useTranslation>["t"];
}) {
  return (
    <SystemTaskFilters
      taskKind={taskKind}
      status={status}
      startedAtFrom={startedAtFrom}
      startedAtTo={startedAtTo}
      filteredCount={filteredCount}
      onTaskKindChange={onTaskKindChange}
      onStatusChange={onStatusChange}
      onStartedAtFromChange={onStartedAtFromChange}
      onStartedAtToChange={onStartedAtToChange}
      t={t}
    />
  );
}

function useSystemTaskRuns({
  page,
  pageSize,
  setPage,
  setPageSize,
  startedAtFrom,
  startedAtTo,
  status,
  taskKind,
}: {
  page: number;
  pageSize: number;
  setPage: (value: number) => void;
  setPageSize: (value: number) => void;
  startedAtFrom?: string;
  startedAtTo?: string;
  status: string;
  taskKind: string;
}) {
  const [items, setItems] = useState<SystemTaskRun[]>([]);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  useEffect(() => {
    let active = true;
    setIsLoading(true);
    fetchSystemTaskRuns({
      taskKind: taskKind || undefined,
      status: status || undefined,
      startedAtFrom,
      startedAtTo,
      page,
      pageSize,
    })
      .then((response) => {
        if (!active) return;
        setItems(response.items);
        setTotal(response.total);
        setPage(response.page);
        setPageSize(response.pageSize);
        setError(null);
      })
      .catch((err) => {
        if (active) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (active) setIsLoading(false);
      });
    return () => {
      active = false;
    };
  }, [page, pageSize, setPage, setPageSize, startedAtFrom, startedAtTo, status, taskKind]);
  return { items, total, error, isLoading };
}

export default function SystemTasksPage() {
  const { t } = useTranslation();
  const [taskKind, setTaskKind] = useState("");
  const [status, setStatus] = useState("");
  const [startedAtFrom, setStartedAtFrom] = useState("");
  const [startedAtTo, setStartedAtTo] = useState("");
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);

  const startedAtFromIso = useMemo(() => toIsoStringOrUndefined(startedAtFrom), [startedAtFrom]);
  const startedAtToIso = useMemo(() => toIsoStringOrUndefined(startedAtTo, true), [startedAtTo]);

  const { items, total, error, isLoading } = useSystemTaskRuns({
    page,
    pageSize,
    setPage,
    setPageSize,
    startedAtFrom: startedAtFromIso,
    startedAtTo: startedAtToIso,
    status,
    taskKind,
  });

  const filteredCount = useMemo(() => total.toLocaleString(), [total]);
  const pageCount = useMemo(() => Math.max(1, Math.ceil(total / pageSize)), [pageSize, total]);

  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-5">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div className="section-heading">
            <h2 className="section-title text-2xl">{t("system.tasks.title")}</h2>
            <p className="section-description max-w-3xl">{t("system.tasks.description")}</p>
          </div>
          <SystemTaskFilterPanel
            taskKind={taskKind}
            status={status}
            startedAtFrom={startedAtFrom}
            startedAtTo={startedAtTo}
            filteredCount={filteredCount}
            onTaskKindChange={(value) => {
              setTaskKind(value);
              setPage(1);
            }}
            onStatusChange={(value) => {
              setStatus(value);
              setPage(1);
            }}
            onStartedAtFromChange={(value) => {
              setStartedAtFrom(value);
              setPage(1);
            }}
            onStartedAtToChange={(value) => {
              setStartedAtTo(value);
              setPage(1);
            }}
            t={t}
          />
        </div>

        {error && items.length > 0 ? (
          <Alert variant="error">{t("system.tasks.loadError", { error })}</Alert>
        ) : null}

        <SystemTaskList items={items} isLoading={isLoading} error={error} t={t} />
        <SystemTaskPagination
          page={page}
          pageCount={pageCount}
          pageSize={pageSize}
          total={total}
          isLoading={isLoading}
          onPageSizeChange={(value) => {
            setPageSize(value);
            setPage(1);
          }}
          onPrevious={() => setPage((current) => Math.max(1, current - 1))}
          onNext={() => setPage((current) => Math.min(pageCount, current + 1))}
          t={t}
        />
      </div>
    </section>
  );
}
