import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { ListBodyState } from "../../features/shared/ListBodyState";
import type { useTranslation } from "../../i18n";
import type { SystemTaskRun } from "../../lib/api";

const TASK_PAGE_SIZE_OPTIONS = [10, 20, 50, 100].map((value) => ({
  value: String(value),
  label: String(value),
}));

function statusTone(status: string): string {
  switch (status) {
    case "success":
      return "text-success";
    case "failed":
      return "text-error";
    case "skipped":
      return "text-warning";
    default:
      return "text-info";
  }
}

type Translation = ReturnType<typeof useTranslation>["t"];

export function SystemTaskFilters({
  taskKind,
  status,
  startedAtFrom,
  startedAtTo,
  filteredCount,
  onTaskKindChange,
  onStatusChange,
  onStartedAtFromChange,
  onStartedAtToChange,
  t,
}: {
  taskKind: string;
  status: string;
  startedAtFrom: string;
  startedAtTo: string;
  filteredCount: string;
  onTaskKindChange: (value: string) => void;
  onStatusChange: (value: string) => void;
  onStartedAtFromChange: (value: string) => void;
  onStartedAtToChange: (value: string) => void;
  t: Translation;
}) {
  return (
    <div className="grid gap-3 min-[769px]:grid-cols-2 xl:grid-cols-5">
      <Input
        value={taskKind}
        onChange={(event) => {
          onTaskKindChange(event.target.value);
        }}
        placeholder={t("system.tasks.filters.taskKindPlaceholder")}
      />
      <SelectField
        value={status}
        onValueChange={(value) => {
          onStatusChange(value);
        }}
        options={[
          { value: "", label: t("system.tasks.filters.allStatuses") },
          { value: "success", label: "success" },
          { value: "failed", label: "failed" },
          { value: "skipped", label: "skipped" },
          { value: "running", label: "running" },
        ]}
      />
      <label className="space-y-1">
        <span className="text-xs font-medium text-base-content/65">
          {t("system.tasks.filters.startedAtFrom")}
        </span>
        <Input
          type="datetime-local"
          value={startedAtFrom}
          onChange={(event) => {
            onStartedAtFromChange(event.target.value);
          }}
        />
      </label>
      <label className="space-y-1">
        <span className="text-xs font-medium text-base-content/65">
          {t("system.tasks.filters.startedAtTo")}
        </span>
        <Input
          type="datetime-local"
          value={startedAtTo}
          onChange={(event) => {
            onStartedAtToChange(event.target.value);
          }}
        />
      </label>
      <div className="flex min-h-10 items-center rounded-xl border border-base-300/75 bg-base-100/72 px-3 text-sm text-base-content/70">
        {t("system.tasks.filters.count", { count: filteredCount })}
      </div>
    </div>
  );
}

export function SystemTaskList({
  items,
  isLoading,
  error,
  t,
}: {
  items: SystemTaskRun[];
  isLoading: boolean;
  error: string | null;
  t: Translation;
}) {
  return (
    <div className="grid gap-4" data-testid="system-tasks-list">
      {isLoading && items.length === 0 ? (
        <ListBodyState
          variant="loading"
          title={t("system.tasks.loading")}
          testId="system-tasks-loading"
        />
      ) : error && items.length === 0 ? (
        <ListBodyState
          variant="error"
          title={t("system.tasks.loadError", { error })}
          testId="system-tasks-error"
        />
      ) : null}
      {items.map((item) => (
        <Card key={item.id} className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
          <CardHeader className="gap-2 border-b border-base-300/70 pb-4">
            <div className="flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
              <div>
                <CardTitle className="text-base font-semibold">{item.taskKind}</CardTitle>
                <CardDescription>
                  {t("system.tasks.meta", {
                    trigger: item.triggerKind,
                    startedAt: item.startedAt,
                  })}
                </CardDescription>
              </div>
              <div
                className={`text-sm font-semibold uppercase tracking-[0.14em] ${statusTone(item.status)}`}
              >
                {item.status}
              </div>
            </div>
          </CardHeader>
          <CardContent className="space-y-2 pt-4 text-sm">
            {item.summary ? <div>{item.summary}</div> : null}
            {item.detail ? <div className="text-base-content/68">{item.detail}</div> : null}
            <div className="text-xs text-base-content/55">
              {t("system.tasks.duration", {
                duration: item.durationMs == null ? "—" : `${item.durationMs} ms`,
                finishedAt: item.finishedAt ?? "—",
              })}
            </div>
          </CardContent>
        </Card>
      ))}
      {!isLoading && !error && items.length === 0 ? (
        <ListBodyState
          variant="empty"
          title={t("system.tasks.empty")}
          testId="system-tasks-empty"
        />
      ) : null}
    </div>
  );
}

export function SystemTaskPagination({
  page,
  pageCount,
  pageSize,
  total,
  isLoading,
  onPageSizeChange,
  onPrevious,
  onNext,
  t,
}: {
  page: number;
  pageCount: number;
  pageSize: number;
  total: number;
  isLoading: boolean;
  onPageSizeChange: (value: number) => void;
  onPrevious: () => void;
  onNext: () => void;
  t: Translation;
}) {
  return (
    <div
      className="flex flex-col gap-3 border-t border-base-300/70 pt-4 sm:flex-row sm:items-end sm:justify-between"
      data-testid="system-tasks-pagination"
    >
      <div className="text-sm text-base-content/70">
        {t("system.tasks.pagination.summary", {
          page,
          pageCount,
          total: total.toLocaleString(),
        })}
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center justify-between gap-2 rounded-xl border border-base-300/70 bg-base-100/55 px-3 py-2 sm:justify-start">
          <span className="text-sm font-medium text-base-content/65">
            {t("system.tasks.pagination.pageSize")}
          </span>
          <SelectField
            className="w-[7rem] min-w-[7rem]"
            value={String(pageSize)}
            options={TASK_PAGE_SIZE_OPTIONS}
            size="sm"
            triggerClassName="h-11 rounded-xl border-base-300/90 bg-base-100 px-3 text-sm lg:h-10"
            aria-label={t("system.tasks.pagination.pageSize")}
            onValueChange={(value) => {
              onPageSizeChange(Number(value));
            }}
          />
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-11 rounded-xl px-4 lg:h-10"
            onClick={onPrevious}
            disabled={isLoading || page <= 1}
          >
            {t("system.tasks.pagination.previous")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-11 rounded-xl px-4 lg:h-10"
            onClick={onNext}
            disabled={isLoading || page >= pageCount}
          >
            {t("system.tasks.pagination.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}
