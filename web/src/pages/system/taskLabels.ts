import type { ManagedTask } from "../../lib/api";

export function managedTaskTriggerLabel(
  task: Pick<ManagedTask, "isManual" | "triggerMode" | "cronExpr">,
): string {
  if (task.isManual || task.triggerMode === "manual") return "手动";
  if (task.cronExpr?.trim()) return "UTC 定时";
  switch (task.triggerMode) {
    case "interval":
      return "固定间隔";
    case "event":
      return "事件触发";
    case "startup":
      return "启动触发";
    default:
      return task.triggerMode || "未设置";
  }
}

export function managedTaskPhaseLabel(phase?: string | null): string {
  switch (phase) {
    case "processing":
      return "处理中";
    case "manual":
      return "手动执行";
    case "pending":
      return "等待执行";
    case "completed":
      return "已完成";
    case "paused":
      return "已暂停";
    default:
      return phase || "未知";
  }
}

export function managedTaskFreshnessLabel(freshness?: string | null): string {
  switch (freshness) {
    case "fresh":
      return "新鲜";
    case "stale":
      return "已过期";
    case "missing":
      return "缺失";
    default:
      return freshness || "未知";
  }
}

export function managedTaskRunStatusLabel(status: string): string {
  switch (status) {
    case "success":
      return "成功";
    case "running":
      return "运行中";
    case "requested":
      return "已请求";
    case "failed":
      return "失败";
    case "skipped":
      return "已跳过";
    default:
      return status;
  }
}

export function managedTaskCompletionLabel(completion?: string | null): string {
  switch (completion) {
    case "completed":
      return "本轮完成";
    case "partial":
      return "部分完成";
    case "deferred":
      return "已延期";
    case "failed":
      return "本轮失败";
    default:
      return "未知";
  }
}

export function managedTaskScheduleSourceLabel(source?: string | null): string {
  switch (source) {
    case "default":
      return "默认计划";
    case "override":
      return "管理员覆盖";
    default:
      return source || "未知";
  }
}

export function managedTaskNextTriggerLabel(task: ManagedTask): string {
  if (!task.enabled) return "已停用";
  const nextTriggerAt = task.nextTriggerAt ?? task.effectiveSchedule?.nextTriggerAt;
  if (nextTriggerAt) {
    const timestamp = Date.parse(nextTriggerAt);
    if (!Number.isNaN(timestamp)) {
      const formatted = new Intl.DateTimeFormat("zh-CN", {
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
        timeZone: "UTC",
      }).format(new Date(timestamp));
      return `${formatted} UTC`;
    }
  }
  if (task.isManual || task.triggerMode === "manual") return "手动触发";
  if (task.triggerMode === "event") return "等待事件";
  if (task.triggerMode === "startup") return "等待启动触发";
  return "未设置";
}
