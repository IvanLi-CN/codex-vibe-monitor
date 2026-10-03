import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/card";
import { Input } from "../../components/ui/input";
import { TaskWorkloadSummary, TaskWorkloadTrend } from "../../features/system/TaskWorkloadTrend";
import { useSubscriptionTopic } from "../../hooks/useSubscriptionTopic";
import {
  type CurrentTaskExecution,
  fetchManagedTask,
  type ManagedTaskDetail,
  runManagedTaskNow,
  type TaskRuntimeSnapshot,
  updateManagedTask,
} from "../../lib/api";
import {
  managedTaskCompletionLabel,
  managedTaskExecutionClassLabel,
  managedTaskFreshnessLabel,
  managedTaskNextTriggerLabel,
  managedTaskPhaseLabel,
  managedTaskRunStatusLabel,
  managedTaskScheduleSourceLabel,
  managedTaskTriggerLabel,
} from "./taskLabels";

function formatDuration(ms?: number | null): string {
  if (ms == null) return "—";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

function formatStartedAt(value?: string | null): string {
  if (!value) return "未知";
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

type ScheduleKind = "interval" | "cron";

function preferNewerWorkloadTrend(
  current: ManagedTaskDetail | null,
  incoming: ManagedTaskDetail,
): ManagedTaskDetail {
  const currentRevision = current?.workloadTrend?.revision ?? -1;
  const incomingRevision = incoming.workloadTrend?.revision ?? -1;
  return currentRevision > incomingRevision
    ? { ...incoming, workloadTrend: current?.workloadTrend }
    : incoming;
}

export default function SystemTaskDetailPage() {
  const { taskKey = "" } = useParams();
  const [detail, setDetail] = useState<ManagedTaskDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [intervalSecs, setIntervalSecs] = useState("");
  const [cronExpr, setCronExpr] = useState("");
  const [scheduleKind, setScheduleKind] = useState<ScheduleKind>("interval");
  const [saving, setSaving] = useState(false);
  const [runningNow, setRunningNow] = useState(false);
  const [activeRuntime, setActiveRuntime] = useState<CurrentTaskExecution | null>(null);
  const [runtimeSampleClock, setRuntimeSampleClock] = useState<number | null>(null);
  const [runtimeSampleElapsedMs, setRuntimeSampleElapsedMs] = useState<number | null>(null);
  const [runtimeNow, setRuntimeNow] = useState(() => window.performance.now());
  const runtimeTopic = useSubscriptionTopic<TaskRuntimeSnapshot>(
    { topic: "system.managed-tasks.runtime" },
    Boolean(taskKey),
  );
  const detailTopic = useSubscriptionTopic<ManagedTaskDetail>(
    { topic: "system.managed-tasks.detail", params: { taskKey } },
    Boolean(taskKey),
  );

  useEffect(() => {
    if (!taskKey) return;
    setDetail(null);
    setError(null);
    fetchManagedTask(taskKey)
      .then((next) => {
        setDetail((current) => preferNewerWorkloadTrend(current, next));
        setIntervalSecs(next.task.intervalSecs == null ? "" : String(next.task.intervalSecs));
        setCronExpr(next.task.cronExpr ?? "");
        setScheduleKind(next.task.cronExpr?.trim() ? "cron" : "interval");
      })
      .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
  }, [taskKey]);

  useEffect(() => {
    const snapshot = runtimeTopic.data;
    if (runtimeTopic.error) {
      setActiveRuntime(null);
      setRuntimeSampleElapsedMs(null);
      setRuntimeSampleClock(null);
      return;
    }
    if (!snapshot) return;
    const next =
      snapshot.activeRuns.find(
        (run) => run.taskKey === taskKey || run.activeChildTaskKey === taskKey,
      ) ?? null;
    setActiveRuntime(next);
    setRuntimeSampleElapsedMs(
      next
        ? next.elapsedMs + Math.max(0, Date.now() - (runtimeTopic.lastReceivedAt ?? Date.now()))
        : null,
    );
    setRuntimeSampleClock(window.performance.now());
  }, [taskKey, runtimeTopic.data, runtimeTopic.error, runtimeTopic.lastReceivedAt]);

  useEffect(() => {
    const incoming = detailTopic.data;
    if (!incoming || incoming.task.taskKey !== taskKey) return;
    setDetail((current) => preferNewerWorkloadTrend(current, incoming));
  }, [detailTopic.data, taskKey]);

  useEffect(() => {
    if (!activeRuntime) return;
    const elapsedTimer = window.setInterval(() => setRuntimeNow(window.performance.now()), 1000);
    return () => window.clearInterval(elapsedTimer);
  }, [activeRuntime]);

  const hasActiveRun = activeRuntime != null;
  const displayedElapsedMs =
    activeRuntime && runtimeSampleElapsedMs != null && runtimeSampleClock != null
      ? runtimeSampleElapsedMs + Math.max(0, runtimeNow - runtimeSampleClock)
      : activeRuntime?.elapsedMs;

  const runtimeError = runtimeTopic.error;
  const runtimeUnavailable = runtimeError != null;
  if (!detail) {
    const chartState = error ? "error" : "loading";
    return (
      <section className="surface-panel surface-panel-body gap-5">
        {error ? (
          <Alert variant="error">任务观测不可用：{error}</Alert>
        ) : (
          <div className="text-sm text-base-content/65">正在读取任务详情…</div>
        )}
        <TaskWorkloadTrend taskKey={taskKey} state={chartState} error={error} />
      </section>
    );
  }

  const { task, progress, recentRuns, performance } = detail;
  const nextCatchupAt = progress?.nextCatchupAt ?? task.nextCatchupAt;
  const save = async (payload: {
    enabled?: boolean;
    intervalSecs?: number | null;
    cronExpr?: string | null;
  }) => {
    setSaving(true);
    try {
      const next = await updateManagedTask(task.taskKey, payload);
      setDetail((current) => preferNewerWorkloadTrend(current, next));
      setIntervalSecs(next.task.intervalSecs == null ? "" : String(next.task.intervalSecs));
      setCronExpr(next.task.cronExpr ?? "");
      setScheduleKind(next.task.cronExpr?.trim() ? "cron" : "interval");
      setError(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };
  const runNow = async () => {
    setRunningNow(true);
    try {
      const next = await runManagedTaskNow(task.taskKey);
      setDetail((current) => preferNewerWorkloadTrend(current, next));
      setError(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setRunningNow(false);
    }
  };

  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-5">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="section-heading">
            <Link to="/system/tasks" className="text-xs text-primary">
              返回任务目录
            </Link>
            <h2 className="section-title text-2xl">{task.title}</h2>
            <p className="section-description">{task.description}</p>
            <p className="text-xs text-base-content/55">任务标识：{task.taskKey}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={saving || runningNow || hasActiveRun}
              onClick={() => void runNow()}
            >
              {runningNow ? "请求中…" : hasActiveRun ? "运行中" : "立即运行"}
            </Button>
            <Button
              type="button"
              disabled={saving || runningNow}
              onClick={() => void save({ enabled: !task.enabled })}
            >
              {saving ? "保存中…" : task.enabled ? "停用任务" : "启用任务"}
            </Button>
          </div>
        </div>
        {error ? <Alert variant="error">{error}</Alert> : null}
        {!error && runtimeError ? <Alert variant="error">{runtimeError}</Alert> : null}
        <Card>
          <CardHeader>
            <CardTitle className="text-base">调度与观测</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            {runtimeUnavailable ? (
              <div className="rounded-lg border border-warning/30 bg-warning/5 p-3 md:col-span-2 xl:col-span-4">
                <div className="text-xs text-base-content/60">当前实际工作</div>
                <div className="mt-1 font-medium">未知</div>
                <div className="mt-1 text-xs text-base-content/60">
                  最近观测失败，等待下一次刷新。
                </div>
              </div>
            ) : activeRuntime ? (
              <div className="rounded-lg border border-primary/25 bg-primary/5 p-3 md:col-span-2 xl:col-span-4">
                <div className="text-xs text-base-content/60">当前实际工作</div>
                <div className="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-sm font-medium">
                  <span>{managedTaskPhaseLabel(activeRuntime.phase)}</span>
                  <span>实际开始：{formatStartedAt(activeRuntime.startedAt)}</span>
                  <span>用时：{formatDuration(displayedElapsedMs)}</span>
                  <span>级别：{managedTaskExecutionClassLabel(activeRuntime.executionClass)}</span>
                </div>
                {activeRuntime.activeChildTitle ? (
                  <div className="mt-1 text-xs text-primary">
                    当前子任务：{activeRuntime.activeChildTitle}
                  </div>
                ) : null}
              </div>
            ) : null}
            <div>
              <div className="text-xs text-base-content/60">触发方式</div>
              <div className="mt-1 font-medium">{managedTaskTriggerLabel(task)}</div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">生效计划</div>
              <div className="mt-1 font-medium">{task.effectivePolicy ?? "未知"}</div>
              <div className="mt-1 text-xs text-base-content/60">
                来源：{task.policySource ?? "未知"}
              </div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">阶段</div>
              <div className="mt-1 font-medium">{managedTaskPhaseLabel(progress?.phase)}</div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">观测新鲜度</div>
              <div className="mt-1 font-medium">
                {managedTaskFreshnessLabel(progress?.freshness)}
              </div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">下次巡检</div>
              <div className="mt-1 font-medium">
                {progress?.nextInspectionAt
                  ? formatStartedAt(progress.nextInspectionAt)
                  : managedTaskNextTriggerLabel(task)}
              </div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">下次追赶资格</div>
              <div className="mt-1 font-medium">
                {nextCatchupAt ? formatStartedAt(nextCatchupAt) : "无"}
              </div>
              <div className="mt-1 text-xs text-base-content/60">
                {progress?.catchupState === "scheduled"
                  ? `原因：${task.catchupReason ?? progress.waitReason ?? "积压仍在"}`
                  : progress?.catchupState === "disabled"
                    ? "任务已停用"
                    : "积压清空后回到巡检计划"}
              </div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">有效计划</div>
              <div className="mt-1 font-medium">
                {managedTaskScheduleSourceLabel(task.effectiveSchedule?.source)}
                {task.effectiveSchedule?.intervalSecs != null
                  ? ` · ${task.effectiveSchedule.intervalSecs}s`
                  : ""}
              </div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">等待原因</div>
              <div className="mt-1 font-medium">{progress?.waitReason ?? "未知"}</div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">数据范围</div>
              <div className="mt-1 break-all font-medium">{progress?.sourceScope ?? "未知"}</div>
            </div>
            <div>
              <div className="text-xs text-base-content/60">最近进度</div>
              <div className="mt-1 font-medium">{progress?.lastProgressAt ?? "未知"}</div>
            </div>
            <div className="md:col-span-2 xl:col-span-4">
              <div className="text-xs text-base-content/60">检查点</div>
              <div className="mt-1 break-all font-medium">{progress?.checkpoint ?? "未知"}</div>
            </div>
            {progress?.stages?.length ? (
              <div className="grid gap-2 border-t border-base-300/60 pt-3 md:col-span-2 xl:col-span-4 sm:grid-cols-3">
                {progress.stages.map((stage) => (
                  <div
                    key={stage.name}
                    className="rounded-md border border-base-300/60 p-2 text-sm"
                  >
                    <div className="font-medium">{stage.name}</div>
                    <div className="text-xs text-base-content/65">{stage.status}</div>
                    {stage.waitReason ? (
                      <div className="text-xs text-base-content/65">等待：{stage.waitReason}</div>
                    ) : null}
                  </div>
                ))}
              </div>
            ) : null}
            {!task.isManual && task.scheduleEditable ? (
              <>
                <fieldset className="space-y-2 md:col-span-2 xl:col-span-4">
                  <legend className="text-sm font-medium">计划方式</legend>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant={scheduleKind === "interval" ? "default" : "outline"}
                      disabled={saving}
                      onClick={() => setScheduleKind("interval")}
                    >
                      固定间隔
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant={scheduleKind === "cron" ? "default" : "outline"}
                      disabled={saving}
                      onClick={() => setScheduleKind("cron")}
                    >
                      UTC crontab
                    </Button>
                  </div>
                  <p className="text-xs text-base-content/60">
                    固定间隔和 UTC crontab 只能选择一种。
                  </p>
                </fieldset>
                {scheduleKind === "interval" ? (
                  <label className="space-y-1 text-sm">
                    <span>间隔（秒）</span>
                    <Input
                      type="number"
                      min={60}
                      value={intervalSecs}
                      onChange={(event) => setIntervalSecs(event.target.value)}
                    />
                  </label>
                ) : (
                  <label className="space-y-1 text-sm md:col-span-2">
                    <span>UTC crontab</span>
                    <Input
                      value={cronExpr}
                      onChange={(event) => setCronExpr(event.target.value)}
                      placeholder="*/5 * * * *"
                    />
                  </label>
                )}
                <div className="flex items-end">
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      disabled={saving}
                      onClick={() =>
                        void save(
                          scheduleKind === "interval"
                            ? {
                                intervalSecs: intervalSecs ? Number(intervalSecs) : null,
                                cronExpr: null,
                              }
                            : { intervalSecs: null, cronExpr: cronExpr.trim() || null },
                        )
                      }
                    >
                      保存调度
                    </Button>
                    {task.intervalSecs != null || task.cronExpr?.trim() ? (
                      <Button
                        type="button"
                        variant="ghost"
                        disabled={saving}
                        onClick={() => void save({ intervalSecs: null, cronExpr: null })}
                      >
                        恢复默认
                      </Button>
                    ) : null}
                  </div>
                </div>
              </>
            ) : !task.isManual ? (
              <div className="md:col-span-2 xl:col-span-4 rounded-lg border border-base-300/70 bg-base-100/35 p-3 text-sm">
                <div className="font-medium">计划由 worker 规则管理</div>
                <div className="mt-1 text-base-content/65">
                  {task.scheduleCapabilityReason ?? "当前任务只读展示生效策略。"}
                </div>
                {task.intervalSecs != null || task.cronExpr?.trim() ? (
                  <Button
                    type="button"
                    variant="ghost"
                    className="mt-2"
                    disabled={saving}
                    onClick={() => void save({ intervalSecs: null, cronExpr: null })}
                  >
                    恢复默认
                  </Button>
                ) : null}
              </div>
            ) : null}
          </CardContent>
        </Card>
        <TaskWorkloadSummary trend={detail.workloadTrend} />
        <TaskWorkloadTrend
          taskKey={task.taskKey}
          capabilities={task.measurementCapabilities}
          trend={detail.workloadTrend}
          retentionTrend={detail.retentionBacklogTrend}
          state="ready"
        />
        <Card>
          <CardHeader>
            <CardTitle className="text-base">最近运行</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {recentRuns.length === 0 ? (
              <div className="text-sm text-base-content/60">暂无运行记录</div>
            ) : (
              recentRuns.map((run) => (
                <div
                  key={run.id}
                  className="grid gap-1 border-b border-base-300/60 pb-3 text-sm last:border-0"
                >
                  <div className="flex flex-wrap justify-between gap-2">
                    <span>{run.startedAt}</span>
                    <span className="font-medium">
                      {managedTaskRunStatusLabel(run.status)} · {formatDuration(run.durationMs)}
                    </span>
                  </div>
                  <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-base-content/60">
                    {run.triggerKind ? <span>触发：{run.triggerKind}</span> : null}
                    {run.processedCount != null ? <span>处理：{run.processedCount}</span> : null}
                    {run.updatedCount != null ? <span>更新：{run.updatedCount}</span> : null}
                  </div>
                  {run.summary ? <div>{run.summary}</div> : null}
                  {run.completion ? (
                    <div className="text-xs text-base-content/65">
                      完成度：{managedTaskCompletionLabel(run.completion)}
                      {run.coreCompletion
                        ? ` · 归档核心：${managedTaskCompletionLabel(run.coreCompletion)}`
                        : ""}
                    </div>
                  ) : null}
                  {run.details?.waitReason ? (
                    <div className="text-xs text-base-content/65">
                      等待：{String(run.details.waitReason)}
                    </div>
                  ) : null}
                  {run.details?.promptCacheStats ? (
                    <div className="text-xs text-base-content/65">
                      Prompt 缓存统计：
                      {run.details.promptCacheStats.state === "available"
                        ? "可用"
                        : run.details.promptCacheStats.state === "unavailable"
                          ? `暂不可用（积压 ${String(run.details.promptCacheStats.pending ?? "未知")}）`
                          : "未知"}
                    </div>
                  ) : null}
                  {run.errorDetail ? <div className="text-error">{run.errorDetail}</div> : null}
                </div>
              ))
            )}
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-base">性能指标（性能库）</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-3 sm:grid-cols-2 xl:grid-cols-6">
            {[
              ["运行次数", performance?.runCount ?? "未知"],
              ["成功次数", performance?.successCount ?? "未知"],
              ["失败次数", performance?.failureCount ?? "未知"],
              ["平均用时", formatDuration(performance?.averageDurationMs)],
              ["最近用时", formatDuration(performance?.latestDurationMs)],
              [
                "覆盖率",
                performance?.coverage == null
                  ? "未知"
                  : `${(performance.coverage * 100).toFixed(1)}%`,
              ],
            ].map(([label, value]) => (
              <div key={label} className="rounded-md border border-base-300/60 p-3">
                <div className="text-xs text-base-content/60">{label}</div>
                <div className="mt-1 font-semibold">{value}</div>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
