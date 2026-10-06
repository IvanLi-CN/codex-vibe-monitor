import type { JSX } from "react";
import type { ManagedTaskRunDetails } from "../../lib/api";

const DATASET_LABELS: Record<string, string> = {
  codex_invocations: "调用记录",
  pool_upstream_request_attempts: "上游尝试",
  forward_proxy_attempts: "代理尝试",
  codex_quota_snapshots: "配额快照",
  codex_invocation_details: "调用详情裁剪",
};

function rate(value?: number | null): string {
  return value == null ? "未知" : `${value.toFixed(2)} 条/s`;
}

function duration(value?: number | null): string {
  return value == null ? "未知" : `${(value / 1000).toFixed(2)} s`;
}

export function RetentionRunThroughput({
  details,
}: {
  details?: ManagedTaskRunDetails | null;
}): JSX.Element {
  const batches = details?.archiveBatches;
  return (
    <section className="space-y-2 text-xs" aria-label="归档吞吐">
      <div className={details?.timeoutCount ? "text-warning" : "text-base-content/65"}>
        超时次数：{details?.timeoutCount ?? "未知"}
        {details?.timeoutCount ? " · 超时兜底已触发，需检查批次规划" : ""}
      </div>
      {batches == null ? (
        <div className="text-base-content/60">归档吞吐：未知（本次记录未提供）</div>
      ) : batches.length === 0 ? (
        <div className="text-base-content/60">本次未开始归档批次</div>
      ) : (
        batches.map((batch) => (
          <div
            key={`${batch.dataset}:${batch.monthKey}`}
            className="rounded-md border border-base-300/60 p-3"
          >
            <div className="mb-2 font-medium">
              {DATASET_LABELS[batch.dataset] ?? batch.dataset} · {batch.monthKey}
            </div>
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3 xl:grid-cols-6">
              {[
                [
                  "已提交 / 批次",
                  `${batch.committedRows ?? "未知"} / ${batch.batchRows ?? "未知"} 条`,
                ],
                ["服务速率", rate(batch.committedRowsPerSecond)],
                ["到达速率（24h）", rate(batch.arrivalRowsPerSecond)],
                [
                  "服务倍率",
                  batch.serviceRateMultiple == null
                    ? "未知"
                    : `${batch.serviceRateMultiple.toFixed(1)}×`,
                ],
                ["文件准备", duration(batch.filePrepareMs)],
                ["锁等待", duration(batch.lockWaitMs)],
              ].map(([label, value]) => (
                <div key={label}>
                  <dt className="text-base-content/60">{label}</dt>
                  <dd className="mt-1 font-medium tabular-nums">{value}</dd>
                </div>
              ))}
            </dl>
            {batch.smallBatchReason ? (
              <div className="mt-2 text-base-content/60">
                小批次原因：
                {batch.smallBatchReason === "tail_config_or_payload_limit"
                  ? "尾批、配置或数据大小限制"
                  : batch.smallBatchReason}
              </div>
            ) : null}
          </div>
        ))
      )}
      {batches?.length ? (
        <div className="text-base-content/55">
          服务速率 = 本批已提交条数 / 本次任务耗时；到达速率按最近 24 小时新增记录计算。
        </div>
      ) : null}
    </section>
  );
}
