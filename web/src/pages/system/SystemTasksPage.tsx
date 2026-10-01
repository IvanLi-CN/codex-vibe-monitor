import { type JSX, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { SelectField } from "../../components/ui/select-field";
import { ListBodyState } from "../../features/shared/ListBodyState";
import {
  type CurrentTaskExecution,
  fetchManagedTaskRuntime,
  fetchManagedTasks,
  type ManagedTask,
  type TaskRuntimeSnapshot,
} from "../../lib/api";
import { managedTaskExecutionClassLabel, managedTaskTriggerLabel } from "./taskLabels";

type EnabledFilter = "all" | "enabled" | "disabled";
const triggerOptions = ["manual", "interval", "cron", "event", "startup", "adaptive"] as const;

function formatElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return hours > 0
    ? `${hours} 小时 ${String(minutes).padStart(2, "0")} 分`
    : `${minutes} 分 ${String(seconds).padStart(2, "0")} 秒`;
}

function formatUtc(value: string): string {
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return "未知";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(new Date(timestamp));
}

function triggerMatches(task: ManagedTask, trigger: string): boolean {
  if (trigger === "cron") return Boolean(task.cronExpr?.trim());
  return (task.triggerKinds ?? [task.triggerMode]).includes(trigger);
}

function RunningTask({
  run,
  receivedAt,
}: {
  run: CurrentTaskExecution;
  receivedAt: number;
}): JSX.Element {
  const [elapsed, setElapsed] = useState(run.elapsedMs);
  useEffect(() => {
    setElapsed(run.elapsedMs);
    const timer = window.setInterval(() => {
      setElapsed(run.elapsedMs + Math.max(0, performance.now() - receivedAt));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [receivedAt, run.elapsedMs]);
  return (
    <div className="grid gap-3 rounded-lg border border-primary/25 bg-primary/5 p-4 sm:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)_auto_auto] sm:items-center">
      <div className="min-w-0">
        <div className="truncate font-semibold">{run.title}</div>
        <div className="mt-1 truncate text-xs text-base-content/60">{run.taskKey}</div>
        {run.activeChildTitle ? (
          <div className="mt-1 truncate text-xs text-primary">
            当前子任务：{run.activeChildTitle}
          </div>
        ) : null}
      </div>
      <div className="text-sm">
        <div className="text-xs text-base-content/55">执行／让行级别</div>
        <div className="mt-1 font-medium">{managedTaskExecutionClassLabel(run.executionClass)}</div>
      </div>
      <div className="text-sm">
        <div className="text-xs text-base-content/55">实际开始</div>
        <div className="mt-1 font-medium">{formatUtc(run.startedAt)}</div>
      </div>
      <div className="text-sm sm:text-right">
        <div className="text-xs text-base-content/55">用时</div>
        <div className="mt-1 font-mono font-medium tabular-nums">{formatElapsed(elapsed)}</div>
      </div>
    </div>
  );
}

export default function SystemTasksPage(): JSX.Element {
  const [tasks, setTasks] = useState<ManagedTask[]>([]);
  const [runtime, setRuntime] = useState<TaskRuntimeSnapshot | null>(null);
  const [lastRuntimeObservedAt, setLastRuntimeObservedAt] = useState<string | null>(null);
  const [runtimeReceivedAt, setRuntimeReceivedAt] = useState(() => performance.now());
  const [error, setError] = useState<string | null>(null);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [enabledFilter, setEnabledFilter] = useState<EnabledFilter>("all");
  const [selectedTriggers, setSelectedTriggers] = useState<Set<string>>(
    () => new Set(triggerOptions),
  );
  const inFlight = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const [catalogResult, runtimeResult] = await Promise.allSettled([
        fetchManagedTasks(),
        fetchManagedTaskRuntime(),
      ]);
      if (catalogResult.status === "fulfilled") {
        setTasks(catalogResult.value);
        setError(null);
      } else {
        const message =
          catalogResult.reason instanceof Error
            ? catalogResult.reason.message
            : String(catalogResult.reason);
        setError(message);
      }
      if (runtimeResult.status === "fulfilled") {
        setRuntime(runtimeResult.value);
        setLastRuntimeObservedAt(runtimeResult.value.observedAt);
        setRuntimeReceivedAt(performance.now());
        setRuntimeError(null);
      } else {
        const message =
          runtimeResult.reason instanceof Error
            ? runtimeResult.reason.message
            : String(runtimeResult.reason);
        setRuntime(null);
        setRuntimeError(message);
      }
    } finally {
      setLoading(false);
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    void refresh();
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, 2000);
    return () => {
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.clearInterval(timer);
    };
  }, [refresh]);

  const filteredTasks = useMemo(
    () =>
      tasks.filter((task) => {
        const enabledMatches =
          enabledFilter === "all" || (enabledFilter === "enabled" ? task.enabled : !task.enabled);
        const triggerMatchesSelection = [...selectedTriggers].some((trigger) =>
          triggerMatches(task, trigger),
        );
        return enabledMatches && triggerMatchesSelection;
      }),
    [enabledFilter, selectedTriggers, tasks],
  );

  const toggleTrigger = (trigger: string) => {
    setSelectedTriggers((current) => {
      const next = new Set(current);
      if (next.has(trigger)) next.delete(trigger);
      else next.add(trigger);
      return next;
    });
  };

  return (
    <section className="surface-panel overflow-hidden" data-testid="system-tasks-list">
      <div className="surface-panel-body gap-5">
        <div className="section-heading">
          <h2 className="section-title text-2xl">任务运维</h2>
          <p className="section-description max-w-3xl">数据库维护任务的进度、调度与运行记录。</p>
        </div>
        {error ? <Alert variant="error">任务目录不可用：{error}</Alert> : null}
        <section aria-labelledby="running-tasks-heading" className="space-y-3">
          <div className="flex flex-wrap items-end justify-between gap-2">
            <div>
              <h3 id="running-tasks-heading" className="text-lg font-semibold">
                当前正在工作
              </h3>
              <p className="text-sm text-base-content/60">排队请求和历史记录不会伪装成正在执行。</p>
            </div>
            <span className="text-xs text-base-content/55">
              {runtime
                ? `观测于 ${formatUtc(runtime.observedAt)}`
                : lastRuntimeObservedAt
                  ? `最后观测 ${formatUtc(lastRuntimeObservedAt)}`
                  : "观测未知"}
            </span>
          </div>
          {runtimeError ? <Alert variant="warning">实时运行观测未知：{runtimeError}</Alert> : null}
          {runtime?.activeRuns.length ? (
            <div className="space-y-2">
              {runtime.activeRuns.map((run) => (
                <RunningTask key={run.executionId} run={run} receivedAt={runtimeReceivedAt} />
              ))}
            </div>
          ) : (
            <div className="rounded-lg border border-dashed border-base-300/80 px-4 py-5 text-sm text-base-content/60">
              {runtimeError
                ? "暂时无法确认当前是否有任务正在工作。"
                : "当前没有已观测到的工作任务。"}
            </div>
          )}
        </section>
        <section aria-labelledby="task-catalog-heading" className="space-y-3">
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div>
              <h3 id="task-catalog-heading" className="text-lg font-semibold">
                任务目录
              </h3>
              <p className="text-sm text-base-content/60">按真实触发机制查看生效策略与来源。</p>
            </div>
            <span className="text-sm text-base-content/60">
              显示 {filteredTasks.length} / {tasks.length}
            </span>
          </div>
          <div className="grid gap-3 rounded-lg border border-base-300/70 bg-base-100/35 p-3 lg:grid-cols-[11rem_minmax(0,1fr)]">
            <SelectField
              label="启用状态"
              value={enabledFilter}
              onValueChange={(value) => setEnabledFilter(value as EnabledFilter)}
              options={[
                { value: "all", label: "全部" },
                { value: "enabled", label: "已启用" },
                { value: "disabled", label: "已停用" },
              ]}
              triggerClassName="field-surface rounded-lg border"
            />
            <fieldset className="min-w-0 space-y-2">
              <legend className="text-xs text-base-content/60">触发方式（多选）</legend>
              <div className="flex flex-wrap gap-2">
                {triggerOptions.map((trigger) => (
                  <label
                    key={trigger}
                    className="inline-flex items-center gap-2 rounded-md border border-base-300/70 px-2.5 py-2 text-sm"
                  >
                    <input
                      type="checkbox"
                      checked={selectedTriggers.has(trigger)}
                      onChange={() => toggleTrigger(trigger)}
                    />
                    <span>
                      {managedTaskTriggerLabel({
                        isManual: trigger === "manual",
                        triggerMode: trigger,
                        cronExpr: trigger === "cron" ? "* * * * *" : null,
                        triggerKinds: [trigger],
                      })}
                    </span>
                  </label>
                ))}
              </div>
            </fieldset>
          </div>
          {loading ? <ListBodyState variant="loading" title="正在读取任务目录" /> : null}
          {!loading && filteredTasks.length === 0 ? (
            <ListBodyState variant="empty" title="没有匹配的任务" />
          ) : null}
          <div className="divide-y divide-base-300/60 overflow-hidden rounded-lg border border-base-300/70">
            {filteredTasks.map((task) => (
              <Link
                key={task.taskKey}
                to={`/system/tasks/${encodeURIComponent(task.taskKey)}`}
                className="grid gap-3 bg-base-100/35 px-4 py-4 transition-colors hover:bg-primary/5 md:grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)_minmax(0,1.35fr)_auto] md:items-center"
              >
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="break-words font-semibold">{task.title}</span>
                    <span
                      className={`text-xs font-semibold ${task.enabled ? "text-success" : "text-base-content/50"}`}
                    >
                      {task.enabled ? "已启用" : "已停用"}
                    </span>
                  </div>
                  <div className="mt-1 break-all text-xs text-base-content/55">{task.taskKey}</div>
                </div>
                <div className="text-sm">
                  <div className="text-xs text-base-content/55">触发方式</div>
                  <div className="mt-1">{managedTaskTriggerLabel(task)}</div>
                </div>
                <div className="text-sm">
                  <div className="text-xs text-base-content/55">生效计划</div>
                  <div className="mt-1">{task.effectivePolicy ?? "未知"}</div>
                  <div className="mt-1 text-xs text-base-content/55">
                    来源：{task.policySource ?? "未知"}
                  </div>
                </div>
                <div className="text-sm md:text-right">
                  <div className="text-xs text-base-content/55">级别</div>
                  <div className="mt-1">{managedTaskExecutionClassLabel(task.executionClass)}</div>
                </div>
              </Link>
            ))}
          </div>
        </section>
      </div>
    </section>
  );
}
