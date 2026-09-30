import { useEffect, useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../components/ui/card";
import { Input } from "../../components/ui/input";
import {
  fetchManagedTask,
  type ManagedTaskDetail,
  runManagedTaskNow,
  updateManagedTask,
} from "../../lib/api";
import {
  managedTaskFreshnessLabel,
  managedTaskNextTriggerLabel,
  managedTaskPhaseLabel,
  managedTaskRunStatusLabel,
  managedTaskTriggerLabel,
} from "./taskLabels";

function formatDuration(ms?: number | null): string {
  if (ms == null) return "—";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

type ScheduleKind = "interval" | "cron";

export default function SystemTaskDetailPage() {
  const { taskKey = "" } = useParams();
  const [detail, setDetail] = useState<ManagedTaskDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [intervalSecs, setIntervalSecs] = useState("");
  const [cronExpr, setCronExpr] = useState("");
  const [scheduleKind, setScheduleKind] = useState<ScheduleKind>("interval");
  const [saving, setSaving] = useState(false);
  const [runningNow, setRunningNow] = useState(false);

  useEffect(() => {
    if (!taskKey) return;
    fetchManagedTask(taskKey)
      .then((next) => {
        setDetail(next);
        setIntervalSecs(next.task.intervalSecs == null ? "" : String(next.task.intervalSecs));
        setCronExpr(next.task.cronExpr ?? "");
        setScheduleKind(next.task.cronExpr?.trim() ? "cron" : "interval");
      })
      .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
  }, [taskKey]);

  const progressPercent = useMemo(() => {
    const total = detail?.progress?.total;
    const completed = detail?.progress?.completed;
    if (total == null || completed == null || total <= 0) return null;
    return Math.min(100, Math.max(0, (completed / total) * 100));
  }, [detail]);

  const activeRunStatus = detail?.recentRuns[0]?.status;
  const hasActiveRun = activeRunStatus === "running" || activeRunStatus === "requested";

  useEffect(() => {
    if (!taskKey || !hasActiveRun) return;
    const timer = window.setInterval(() => {
      void fetchManagedTask(taskKey)
        .then(setDetail)
        .catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
    }, 750);
    return () => window.clearInterval(timer);
  }, [hasActiveRun, taskKey]);

  if (error && !detail) return <Alert variant="error">任务观测不可用：{error}</Alert>;
  if (!detail)
    return <div className="surface-panel p-6 text-base-content/65">正在读取任务详情…</div>;

  const { task, progress, recentRuns } = detail;
  const save = async (payload: {
    enabled?: boolean;
    intervalSecs?: number | null;
    cronExpr?: string | null;
  }) => {
    setSaving(true);
    try {
      const next = await updateManagedTask(task.taskKey, payload);
      setDetail(next);
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
      setDetail(await runManagedTaskNow(task.taskKey));
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
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          {[
            ["总量", progress?.total == null ? "—" : progress.total.toLocaleString()],
            ["已完成", progress?.completed == null ? "—" : progress.completed.toLocaleString()],
            ["当前进度", progressPercent == null ? "—" : `${progressPercent.toFixed(1)}%`],
            ["预计剩余", progress?.etaSeconds == null ? "—" : `${progress.etaSeconds}s`],
          ].map(([label, value]) => (
            <Card key={label}>
              <CardHeader className="pb-2">
                <CardTitle className="text-xs text-base-content/60">{label}</CardTitle>
              </CardHeader>
              <CardContent className="text-2xl font-semibold">{value}</CardContent>
            </Card>
          ))}
        </div>
        <Card>
          <CardHeader>
            <CardTitle className="text-base">调度与观测</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
            <div>
              <div className="text-xs text-base-content/60">触发方式</div>
              <div className="mt-1 font-medium">{managedTaskTriggerLabel(task)}</div>
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
              <div className="text-xs text-base-content/60">下次运行</div>
              <div className="mt-1 font-medium">{managedTaskNextTriggerLabel(task)}</div>
            </div>
            <div className="md:col-span-2 xl:col-span-4">
              <div className="text-xs text-base-content/60">检查点</div>
              <div className="mt-1 break-all font-medium">{progress?.checkpoint ?? "未知"}</div>
            </div>
            {!task.isManual ? (
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
                          : {
                              intervalSecs: null,
                              cronExpr: cronExpr.trim() || null,
                            },
                      )
                    }
                  >
                    保存调度
                  </Button>
                </div>
              </>
            ) : null}
          </CardContent>
        </Card>
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
                  {run.errorDetail ? <div className="text-error">{run.errorDetail}</div> : null}
                </div>
              ))
            )}
          </CardContent>
        </Card>
      </div>
    </section>
  );
}
