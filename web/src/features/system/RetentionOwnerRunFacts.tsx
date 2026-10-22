import type { JSX } from "react";
import type { ManagedTaskRunDetails } from "../../lib/api";

function count(value?: number | null): string {
  return value == null ? "未知" : value.toLocaleString("zh-CN");
}

function resultLabel(value?: string | null): string {
  switch (value) {
    case "no-work":
      return "无待处理工作";
    case "complete":
      return "完成";
    case "partial":
      return "部分完成";
    case "deferred":
      return "已延期";
    case "failed":
      return "失败";
    default:
      return value || "未知";
  }
}

function text(value?: string | null): string {
  return value?.trim() || "未知";
}

export function RetentionOwnerRunFacts({
  details,
}: {
  details?: ManagedTaskRunDetails | null;
}): JSX.Element | null {
  if (details?.ownershipVersion !== 2) return null;

  const selectedScope = details.selectedScope;
  const rawFileLabel = details.dryRun ? "计划文件" : "释放文件";
  const rawFileCount = details.dryRun
    ? selectedScope?.rawFilesPlanned
    : selectedScope?.rawFilesRemoved;
  const coverage =
    selectedScope?.hasMore == null
      ? "覆盖：未知"
      : selectedScope.hasMore
        ? "覆盖：窗口后仍有候选"
        : "覆盖：本轮窗口未发现更多候选";

  return (
    <section
      aria-label="保留 owner 运行事实"
      className="rounded-md border border-base-300/60 bg-base-200/25 p-3 text-xs"
    >
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 font-medium">
        <span>本任务运行事实</span>
        <span className="text-base-content/60">
          {details.dryRun ? "只读预演" : "实际运行"} · 结果：{resultLabel(details.result)}
        </span>
      </div>
      <dl className="grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3 xl:grid-cols-4">
        {[
          ["范围", text(selectedScope?.scope ?? details.ownerScope)],
          ["单位", text(selectedScope?.unit)],
          ["全局积压", count(details.globalBacklogTotal)],
          ["本轮检查", count(details.discovered)],
          ["本轮选中", count(details.selected ?? selectedScope?.selected)],
          ["本轮提交", count(details.committed ?? selectedScope?.committed)],
          ["批次", count(selectedScope?.batches)],
          [rawFileLabel, count(rawFileCount)],
        ].map(([label, value]) => (
          <div key={label} className="min-w-0">
            <dt className="text-base-content/60">{label}</dt>
            <dd className="mt-1 break-all font-medium tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
      <div className="mt-2 text-base-content/60">{coverage}</div>
    </section>
  );
}
