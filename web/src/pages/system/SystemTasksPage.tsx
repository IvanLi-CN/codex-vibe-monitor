import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { ListBodyState } from "../../features/shared/ListBodyState";
import { fetchManagedTasks, type ManagedTask } from "../../lib/api";
import { managedTaskTriggerLabel } from "./taskLabels";

export default function SystemTasksPage() {
  const [tasks, setTasks] = useState<ManagedTask[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let active = true;
    fetchManagedTasks()
      .then((items) => active && setTasks(items))
      .catch(
        (reason) => active && setError(reason instanceof Error ? reason.message : String(reason)),
      )
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, []);

  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-5">
        <div className="section-heading">
          <h2 className="section-title text-2xl">任务运维</h2>
          <p className="section-description max-w-3xl">数据库维护任务的进度、调度与运行记录。</p>
        </div>
        {error ? <Alert variant="error">维护库观测不可用：{error}</Alert> : null}
        {loading ? <ListBodyState variant="loading" title="正在读取任务目录" /> : null}
        {!loading && tasks.length === 0 ? (
          <ListBodyState variant="empty" title="暂无任务观测" />
        ) : null}
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {tasks.map((task) => (
            <Link
              key={task.taskKey}
              to={`/system/tasks/${encodeURIComponent(task.taskKey)}`}
              className="block w-full min-w-0"
            >
              <Card className="h-full min-w-0 w-full transition-colors hover:border-primary/60">
                <CardHeader className="gap-2">
                  <div className="flex min-w-0 items-start justify-between gap-3">
                    <CardTitle className="min-w-0 break-words text-base">{task.title}</CardTitle>
                    <span
                      className={`shrink-0 text-xs font-semibold ${task.enabled ? "text-success" : "text-base-content/50"}`}
                    >
                      {task.enabled ? "已启用" : "已停用"}
                    </span>
                  </div>
                  <CardDescription>{task.description}</CardDescription>
                </CardHeader>
                <CardContent className="flex min-w-0 items-center justify-between gap-3 text-xs text-base-content/60">
                  <span className="min-w-0 break-all">{task.taskKey}</span>
                  <span className="shrink-0">{managedTaskTriggerLabel(task)}</span>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      </div>
    </section>
  );
}
