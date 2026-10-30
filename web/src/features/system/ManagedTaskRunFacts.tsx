import type { JSX } from "react";
import { useState } from "react";
import { Button } from "../../components/ui/button";
import { Tooltip } from "../../components/ui/tooltip";
import type { ManagedTaskRun } from "../../lib/api";
import { copyText } from "../../lib/clipboard";
import { AppIcon } from "../shared/AppIcon";
import { formatManagedTaskDuration, formatManagedTaskTimestamp } from "./taskRunFacts";

export function ManagedTaskRunFacts({ run }: { run: ManagedTaskRun }): JSX.Element {
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const executionUid = run.executionUid?.trim() || null;
  const fileResourceWaitMs = run.details?.fileResourceWaitMs;
  const hasFileResourceWait =
    typeof fileResourceWaitMs === "number" &&
    Number.isFinite(fileResourceWaitMs) &&
    fileResourceWaitMs >= 0;

  const copyExecutionUid = async () => {
    if (!executionUid) return;
    const result = await copyText(executionUid);
    setCopyState(result.ok ? "copied" : "failed");
  };

  return (
    <section
      aria-label="运行时间与身份"
      className="rounded-md border border-base-300/60 bg-base-200/20 p-3 text-xs"
    >
      <dl className="grid gap-x-4 gap-y-2 sm:grid-cols-2 xl:grid-cols-7">
        <div>
          <dt className="text-base-content/60">触发时间</dt>
          <dd className="mt-1 font-medium tabular-nums">
            {formatManagedTaskTimestamp(run.startedAt)}
          </dd>
        </div>
        <div>
          <dt className="text-base-content/60">实际开始</dt>
          <dd className="mt-1 font-medium tabular-nums">
            {formatManagedTaskTimestamp(run.actualStartedAt)}
          </dd>
        </div>
        <div>
          <dt className="text-base-content/60">实际结束</dt>
          <dd className="mt-1 font-medium tabular-nums">
            {formatManagedTaskTimestamp(run.actualFinishedAt)}
          </dd>
        </div>
        <div>
          <dt className="text-base-content/60">执行用时</dt>
          <dd className="mt-1 font-medium tabular-nums">
            {formatManagedTaskDuration(run.executionDurationMs)}
          </dd>
        </div>
        <div>
          <dt className="text-base-content/60">调度等待</dt>
          <dd className="mt-1 font-medium tabular-nums">
            {formatManagedTaskDuration(run.schedulerWaitMs)}
          </dd>
        </div>
        <div>
          <dt className="text-base-content/60">调度侧资源准入等待</dt>
          <dd className="mt-1 font-medium tabular-nums">
            {formatManagedTaskDuration(run.details?.resourceWaitMs)}
          </dd>
        </div>
        {hasFileResourceWait ? (
          <div>
            <dt className="text-base-content/60">文件资源等待</dt>
            <dd className="mt-1 font-medium tabular-nums">
              {formatManagedTaskDuration(fileResourceWaitMs)}
            </dd>
          </div>
        ) : null}
      </dl>
      <div className="mt-3 flex min-w-0 flex-wrap items-center gap-2">
        <span className="shrink-0 text-base-content/60">执行身份</span>
        {executionUid ? (
          <code className="min-w-0 flex-1 break-all rounded bg-base-200/70 px-1.5 py-1 font-mono text-[11px] text-base-content">
            {executionUid}
          </code>
        ) : (
          <span className="font-medium">运行记录 ID：{run.id}</span>
        )}
        {executionUid ? (
          <Tooltip content="复制执行身份">
            <Button
              type="button"
              size="icon"
              variant="outline"
              aria-label="复制执行身份"
              title="复制执行身份"
              onClick={() => void copyExecutionUid()}
            >
              <AppIcon name="content-copy" className="h-4 w-4" aria-hidden />
            </Button>
          </Tooltip>
        ) : null}
        {copyState !== "idle" ? (
          <span role="status" className="text-base-content/60">
            {copyState === "copied" ? "已复制" : "复制失败，请手动复制"}
          </span>
        ) : null}
      </div>
    </section>
  );
}
