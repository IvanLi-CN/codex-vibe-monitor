import { type JSX, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { SelectField } from "../../components/ui/select-field";
import { ListBodyState } from "../../features/shared/ListBodyState";
import { managedTaskColor } from "../../features/system/managedTaskColor";
import { TaskTimelineChart } from "../../features/system/TaskTimelineChart";
import {
  type CurrentTaskExecution,
  fetchManagedTaskRuntime,
  fetchManagedTasks,
  fetchManagedTaskTimeline,
  type ManagedTask,
  type TaskAdmissionWait,
  type TaskRuntimeSnapshot,
  type TaskTimelineCoverage,
  type TaskTimelineSegment,
} from "../../lib/api";
import { managedTaskExecutionClassLabel, managedTaskTriggerLabel } from "./taskLabels";

type EnabledFilter = "all" | "enabled" | "disabled";
const triggerOptions = ["manual", "interval", "cron", "event", "startup", "adaptive"] as const;
const DAY_MS = 24 * 60 * 60 * 1000;
const FRESHNESS_LIMIT_MS = 6_000;

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

function useDarkColorMode(): boolean {
  if (typeof document === "undefined") return false;
  return (
    document.documentElement.getAttribute("data-color-mode") === "dark" ||
    document.documentElement.getAttribute("data-theme") === "vibe-dark"
  );
}

function getElapsed(baseMs: number, receivedAt: number | null): number {
  if (receivedAt == null || performance.now() - receivedAt > FRESHNESS_LIMIT_MS) return baseMs;
  return baseMs + Math.max(0, performance.now() - receivedAt);
}

function TaskDot({ task, dark }: { task: ManagedTask | undefined; dark: boolean }): JSX.Element {
  return (
    <span
      aria-hidden="true"
      className="size-2 shrink-0 rounded-full"
      style={{ backgroundColor: managedTaskColor(task, dark) }}
    />
  );
}

function RunningTask({
  run,
  task,
  dark,
  elapsed,
  stale,
}: {
  run: CurrentTaskExecution;
  task: ManagedTask | undefined;
  dark: boolean;
  elapsed: number;
  stale: boolean;
}): JSX.Element {
  return (
    <div
      className={`grid gap-x-3 gap-y-1 border-b border-primary/20 px-3 py-2 sm:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)_auto_auto] sm:items-center ${stale ? "bg-base-200/35" : "bg-primary/5"}`}
    >
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <TaskDot task={task} dark={dark} />
          <span className="truncate font-semibold">{run.title}</span>
          {stale ? <span className="shrink-0 text-xs text-warning">状态未知</span> : null}
        </div>
        <div className="mt-1 truncate pl-4 text-xs text-base-content/60">{run.taskKey}</div>
        {run.activeChildTitle ? (
          <div className="mt-1 truncate pl-4 text-xs text-primary">
            {stale ? "最后确认子阶段" : "当前子阶段"}：{run.activeChildTitle}
          </div>
        ) : null}
      </div>
      <div className="text-sm sm:flex sm:items-center sm:gap-2">
        <div className="text-xs text-base-content/55">触发 / 执行级别</div>
        <div className="mt-1 font-medium sm:mt-0">
          {managedTaskTriggerLabel(
            task ?? {
              triggerMode: run.triggerKind,
              isManual: run.triggerKind === "manual",
              cronExpr: null,
            },
          )}
          <span className="px-1 text-base-content/35">/</span>
          {managedTaskExecutionClassLabel(run.executionClass)}
        </div>
      </div>
      <div className="text-sm sm:flex sm:items-center sm:gap-2">
        <div className="text-xs text-base-content/55">实际开始</div>
        <div className="mt-1 font-medium sm:mt-0">{formatUtc(run.startedAt)}</div>
      </div>
      <div className="text-sm sm:flex sm:items-center sm:justify-end sm:gap-2">
        <div className="text-xs text-base-content/55">用时</div>
        <div className="mt-1 font-mono font-medium tabular-nums sm:mt-0">
          {formatElapsed(elapsed)}
        </div>
      </div>
    </div>
  );
}

function WaitingTask({
  title,
  taskKey,
  task,
  dark,
  kind,
  waitingStartedAt,
  waitingMs,
  receivedAt,
  detail,
}: {
  title: string;
  taskKey: string;
  task: ManagedTask | undefined;
  dark: boolean;
  kind: "queue" | "admission";
  waitingStartedAt: string | null | undefined;
  waitingMs: number;
  receivedAt: number | null;
  detail: string;
}): JSX.Element {
  return (
    <div className="grid gap-x-2 gap-y-1 border-b border-base-300/60 px-3 py-2 sm:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)_auto] sm:items-center">
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <TaskDot task={task} dark={dark} />
          <span className="truncate font-medium">{title}</span>
          <span className="shrink-0 text-xs text-base-content/50">
            {kind === "queue" ? "已入队" : "等待准入"}
          </span>
          <span className="hidden min-w-0 truncate text-xs text-base-content/50 sm:inline">
            · {taskKey}
          </span>
        </div>
        <div className="mt-1 truncate pl-4 text-xs text-base-content/55 sm:hidden">{taskKey}</div>
      </div>
      <div className="text-sm text-base-content/70">
        <div>{detail}</div>
        <div className="mt-0.5 text-xs text-base-content/55">
          {kind === "queue" ? "请求时间" : "等待开始"} ·{" "}
          {waitingStartedAt ? formatUtc(waitingStartedAt) : "未知"}
        </div>
      </div>
      <div className="text-sm sm:flex sm:items-center sm:justify-end sm:gap-2">
        <div className="text-xs text-base-content/55">等待时长</div>
        <div className="mt-1 font-mono tabular-nums sm:mt-0">
          {formatElapsed(getElapsed(waitingMs, receivedAt))}
        </div>
      </div>
    </div>
  );
}

function admissionReason(wait: TaskAdmissionWait): string {
  const reason =
    wait.reason === "pressure_cooldown"
      ? "压力冷却让行"
      : wait.reason === "resource_busy"
        ? "资源占用等待"
        : wait.reason;
  return `${reason} · 重试时间 ${wait.retryAt ? formatUtc(wait.retryAt) : "未知"}`;
}

export default function SystemTasksPage(): JSX.Element {
  const [tasks, setTasks] = useState<ManagedTask[]>([]);
  const [runtime, setRuntime] = useState<TaskRuntimeSnapshot | null>(null);
  const [runtimeReceivedAt, setRuntimeReceivedAt] = useState<number | null>(null);
  const [lastRuntimeObservedAt, setLastRuntimeObservedAt] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<TaskTimelineSegment[]>([]);
  const [coverage, setCoverage] = useState<TaskTimelineCoverage[]>([]);
  const [timelineReceivedAt, setTimelineReceivedAt] = useState<number | null>(null);
  const [timelineError, setTimelineError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [runtimeError, setRuntimeError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [now, setNow] = useState(() => Date.now());
  const [enabledFilter, setEnabledFilter] = useState<EnabledFilter>("all");
  const [selectedTriggers, setSelectedTriggers] = useState<Set<string>>(
    () => new Set(triggerOptions),
  );
  const inFlight = useRef(false);
  const timelineRevision = useRef<number | null>(null);

  const loadTimeline = useCallback(async () => {
    let firstPage = await fetchManagedTaskTimeline(
      timelineRevision.current == null ? {} : { afterRevision: timelineRevision.current },
    );
    let replace = timelineRevision.current == null;
    if (firstPage.resetRequired) {
      firstPage = await fetchManagedTaskTimeline();
      replace = true;
    }
    const pages = [firstPage];
    let cursor = firstPage.nextCursor;
    while (cursor) {
      const page = await fetchManagedTaskTimeline({ cursor });
      pages.push(page);
      cursor = page.nextCursor;
    }
    timelineRevision.current = firstPage.watermark;
    return { pages, replace };
  }, []);

  const refresh = useCallback(
    async (includeCatalog = false) => {
      if (inFlight.current) return;
      inFlight.current = true;
      const catalogPromise = includeCatalog ? fetchManagedTasks() : Promise.resolve(undefined);
      try {
        const [catalogResult, runtimeResult, timelineResult] = await Promise.allSettled([
          catalogPromise,
          fetchManagedTaskRuntime(),
          loadTimeline(),
        ]);
        if (catalogResult.status === "fulfilled" && catalogResult.value) {
          setTasks(catalogResult.value);
          setError(null);
        } else if (catalogResult.status === "rejected") {
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
          setRuntimeError(message);
        }
        if (timelineResult.status === "fulfilled") {
          const { pages, replace } = timelineResult.value;
          const received = pages.at(-1);
          const incoming = pages.flatMap((page) => page.segments);
          const retainFrom = Date.now() - DAY_MS;
          setTimeline((current) => {
            const merged = new Map<string, TaskTimelineSegment>();
            if (!replace) {
              for (const segment of current) {
                const end = Date.parse(segment.finishedAt ?? segment.lastObservedAt);
                if (!Number.isFinite(end) || end >= retainFrom)
                  merged.set(segment.segmentId, segment);
              }
            }
            for (const segment of incoming) merged.set(segment.segmentId, segment);
            return [...merged.values()];
          });
          if (received) {
            setCoverage(received.coverage);
            setTimelineReceivedAt(performance.now());
            setTimelineError(null);
          }
        } else {
          const message =
            timelineResult.reason instanceof Error
              ? timelineResult.reason.message
              : String(timelineResult.reason);
          setTimelineError(message);
        }
      } finally {
        setLoading(false);
        inFlight.current = false;
      }
    },
    [loadTimeline],
  );

  useEffect(() => {
    void refresh(true);
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") void refresh(true);
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    const refreshTimer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh(false);
    }, 2000);
    const clockTimer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => {
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.clearInterval(refreshTimer);
      window.clearInterval(clockTimer);
    };
  }, [refresh]);

  const taskByKey = useMemo(() => new Map(tasks.map((task) => [task.taskKey, task])), [tasks]);
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
  const dark = useDarkColorMode();
  const runtimeAge =
    runtimeReceivedAt == null ? Number.POSITIVE_INFINITY : performance.now() - runtimeReceivedAt;
  const runtimeFresh = runtimeAge <= FRESHNESS_LIMIT_MS;
  const visibleRuns = runtime?.activeRuns ?? [];
  const queue = runtime?.queuedRuns ?? [];
  const admissionWaits = runtime?.admissionWaits ?? [];
  const queueAvailable = runtimeFresh && runtime?.queuedRunsAvailable === true;
  const admissionWaitsAvailable = runtimeFresh && runtime?.admissionWaitsAvailable === true;

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

        <section
          aria-labelledby="current-tasks-heading"
          className="overflow-hidden rounded-md border border-base-300/70"
        >
          <div className="flex flex-wrap items-end justify-between gap-2 border-b border-base-300/70 px-3 py-2">
            <div>
              <h3 id="current-tasks-heading" className="text-lg font-semibold">
                当前任务
              </h3>
              <p className="text-sm text-base-content/60">
                执行、dispatcher 队列与准入等待分开显示。
              </p>
            </div>
            <span className="text-xs text-base-content/55">
              {runtimeFresh && runtime
                ? `观测于 ${formatUtc(runtime.observedAt)}`
                : lastRuntimeObservedAt
                  ? `最后确认 ${formatUtc(lastRuntimeObservedAt)} · 动态计时已暂停`
                  : "观测未知"}
            </span>
          </div>
          {runtimeError ? <Alert variant="warning">实时运行观测未知：{runtimeError}</Alert> : null}
          <section aria-labelledby="running-tasks-heading">
            <h4
              id="running-tasks-heading"
              className="border-b border-base-300/60 bg-base-100/35 px-3 py-1.5 text-xs font-semibold"
            >
              {runtimeFresh ? "正在执行" : "执行状态未知"}{" "}
              <span className="font-normal text-base-content/55">
                {runtimeFresh ? visibleRuns.length : "未知"}
              </span>
            </h4>
            {visibleRuns.length ? (
              visibleRuns.map((run) => (
                <RunningTask
                  key={run.executionUid || run.executionId}
                  run={run}
                  task={taskByKey.get(run.taskKey)}
                  dark={dark}
                  elapsed={getElapsed(run.elapsedMs, runtimeReceivedAt)}
                  stale={!runtimeFresh}
                />
              ))
            ) : (
              <div className="px-3 py-3 text-sm text-base-content/60">
                {runtimeFresh
                  ? "当前没有已观测到的工作任务。"
                  : "运行快照已过期，当前是否有任务正在工作未知。"}
              </div>
            )}
          </section>
          <section aria-labelledby="queued-tasks-heading">
            <h4
              id="queued-tasks-heading"
              className="border-y border-base-300/60 bg-base-100/35 px-3 py-1.5 text-xs font-semibold"
            >
              已入队{" "}
              <span className="font-normal text-base-content/55">
                {queueAvailable ? queue.length : "未知"}
              </span>
            </h4>
            {!runtimeFresh ? (
              <div className="px-3 py-3 text-sm text-base-content/60">
                队列快照已过期，当前等待请求未知。
              </div>
            ) : !runtime?.queuedRunsAvailable ? (
              <div className="px-3 py-3 text-sm text-base-content/60">
                dispatcher 队列当前不可用，无法确认等待请求。
              </div>
            ) : queue.length ? (
              queue.map((run) => (
                <WaitingTask
                  key={run.runId}
                  title={run.title}
                  taskKey={run.taskKey}
                  task={taskByKey.get(run.taskKey)}
                  dark={dark}
                  kind="queue"
                  waitingStartedAt={run.requestedAt}
                  waitingMs={run.waitingMs}
                  receivedAt={runtimeReceivedAt}
                  detail={`第 ${run.position} 位 · ${managedTaskTriggerLabel({ triggerMode: run.triggerKind, isManual: run.triggerKind === "manual", cronExpr: null })}`}
                />
              ))
            ) : (
              <div className="px-3 py-3 text-sm text-base-content/60">当前没有已入队请求。</div>
            )}
          </section>
          <section aria-labelledby="admission-waits-heading">
            <h4
              id="admission-waits-heading"
              className="border-y border-base-300/60 bg-base-100/35 px-3 py-1.5 text-xs font-semibold"
            >
              等待准入 / 压力延后{" "}
              <span className="font-normal text-base-content/55">
                {admissionWaitsAvailable ? admissionWaits.length : "未知"}
              </span>
            </h4>
            {!runtimeFresh ? (
              <div className="px-3 py-3 text-sm text-base-content/60">
                准入等待快照已过期，当前等待状态未知。
              </div>
            ) : !runtime?.admissionWaitsAvailable ? (
              <div className="px-3 py-3 text-sm text-base-content/60">
                准入让行记录当前不可用，无法确认等待状态。
              </div>
            ) : admissionWaits.length ? (
              admissionWaits.map((wait) => (
                <WaitingTask
                  key={wait.id}
                  title={wait.title}
                  taskKey={wait.taskKey}
                  task={taskByKey.get(wait.taskKey)}
                  dark={dark}
                  kind="admission"
                  waitingStartedAt={wait.startedAt}
                  waitingMs={wait.waitingMs}
                  receivedAt={runtimeReceivedAt}
                  detail={admissionReason(wait)}
                />
              ))
            ) : (
              <div className="px-3 py-3 text-sm text-base-content/60">
                当前没有已观测到的准入等待。
              </div>
            )}
          </section>
        </section>

        <TaskTimelineChart
          tasks={tasks}
          executions={timeline}
          activeRuns={visibleRuns}
          coverage={coverage}
          nowMs={now}
          runtimeObservedAt={runtime?.observedAt ?? lastRuntimeObservedAt}
          runtimeReceivedAt={runtimeReceivedAt}
        />
        {timelineError ? (
          <Alert variant="warning">
            时间线增量暂不可用，显示最后一次确认的区间：{timelineError}
          </Alert>
        ) : timelineReceivedAt == null && !loading ? (
          <Alert variant="warning">尚无可用的时间线记录，当前区间会以观测缺口呈现。</Alert>
        ) : null}

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
          <div className="grid gap-3 rounded-md border border-base-300/70 bg-base-100/35 p-3 lg:grid-cols-[11rem_minmax(0,1fr)]">
            <SelectField
              label="启用状态"
              value={enabledFilter}
              onValueChange={(value) => setEnabledFilter(value as EnabledFilter)}
              options={[
                { value: "all", label: "全部" },
                { value: "enabled", label: "已启用" },
                { value: "disabled", label: "已停用" },
              ]}
              triggerClassName="field-surface rounded-md border"
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
          <div className="divide-y divide-base-300/60 overflow-hidden rounded-md border border-base-300/70">
            {filteredTasks.map((task) => (
              <Link
                key={task.taskKey}
                to={`/system/tasks/${encodeURIComponent(task.taskKey)}`}
                className="grid gap-3 bg-base-100/35 px-4 py-4 transition-colors hover:bg-primary/5 md:grid-cols-[minmax(0,1.25fr)_minmax(0,1fr)_minmax(0,1.35fr)_auto] md:items-center"
              >
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="flex min-w-0 items-center gap-2">
                      <TaskDot task={task} dark={dark} />
                      <span className="break-words font-semibold">{task.title}</span>
                    </span>
                    <span
                      className={`text-xs font-semibold ${task.enabled ? "text-success" : "text-base-content/50"}`}
                    >
                      {task.enabled ? "已启用" : "已停用"}
                    </span>
                  </div>
                  <div className="mt-1 break-all pl-4 text-xs text-base-content/55">
                    {task.taskKey}
                  </div>
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
