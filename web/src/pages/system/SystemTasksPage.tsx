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
              className="block"
            >
              <Card className="h-full transition-colors hover:border-primary/60">
                <CardHeader className="gap-2">
                  <div className="flex items-start justify-between gap-3">
                    <CardTitle className="text-base">{task.title}</CardTitle>
                    <span
                      className={`text-xs font-semibold ${task.enabled ? "text-success" : "text-base-content/50"}`}
                    >
                      {task.enabled ? "已启用" : "已停用"}
                    </span>
                  </div>
                  <CardDescription>{task.description}</CardDescription>
                </CardHeader>
                <CardContent className="flex items-center justify-between text-xs text-base-content/60">
                  <span>{task.taskKey}</span>
                  <span>{task.isManual ? "手动" : task.triggerMode}</span>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      </div>
    </section>
  );
}
