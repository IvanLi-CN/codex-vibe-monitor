import type { JSX } from "react";
import type { ManagedTaskRunDetails } from "../../lib/api";
import { formatManagedTaskDuration } from "./taskRunFacts";

const DATASET_LABELS: Record<string, string> = {
  codex_invocations: "调用记录",
  pool_upstream_request_attempts: "上游尝试",
  forward_proxy_attempts: "代理尝试",
  codex_quota_snapshots: "配额快照",
  codex_invocation_details: "调用详情裁剪",
};

function rate(value?: number | null): string {
  return value == null || !Number.isFinite(value) || value < 0
    ? "未知"
    : `${value.toFixed(2)} 条/s`;
}

function duration(value?: number | null): string {
  return value == null || !Number.isFinite(value) || value < 0
    ? "未知"
    : `${(value / 1000).toFixed(2)} s`;
}

function serviceRate(
  committedRowsPerSecond: number | null | undefined,
  committedRows: number | null | undefined,
  executionDurationMs: number | null | undefined,
): number | null {
  if (
    committedRowsPerSecond != null &&
    Number.isFinite(committedRowsPerSecond) &&
    committedRowsPerSecond >= 0
  ) {
    return committedRowsPerSecond;
  }
  if (
    committedRows == null ||
    !Number.isFinite(committedRows) ||
    committedRows < 0 ||
    executionDurationMs == null ||
    !Number.isFinite(executionDurationMs) ||
    executionDurationMs <= 0
  ) {
    return null;
  }
  return committedRows / (executionDurationMs / 1000);
}

function serviceRateMultiple(
  authoritativeMultiple: number | null | undefined,
  serviceRateValue: number | null,
  arrivalRowsPerSecond: number | null | undefined,
): number | null {
  if (
    authoritativeMultiple != null &&
    Number.isFinite(authoritativeMultiple) &&
    authoritativeMultiple >= 0
  ) {
    return authoritativeMultiple;
  }
  if (
    serviceRateValue == null ||
    arrivalRowsPerSecond == null ||
    !Number.isFinite(arrivalRowsPerSecond) ||
    arrivalRowsPerSecond <= 0
  ) {
    return null;
  }
  return serviceRateValue / arrivalRowsPerSecond;
}

export function RetentionRunThroughput({
  details,
  executionDurationMs,
  schedulerWaitMs,
}: {
  details?: ManagedTaskRunDetails | null;
  executionDurationMs?: number | null;
  schedulerWaitMs?: number | null;
}): JSX.Element {
  const batches = details?.archiveBatches;
  const canUseRunFallback = batches?.length === 1;
  return (
    <section className="space-y-2 text-xs" aria-label="归档吞吐">
      <div className={details?.timeoutCount ? "text-warning" : "text-base-content/65"}>
        超时次数：{details?.timeoutCount ?? "未知"}
        {details?.timeoutCount ? " · 超时兜底已触发，需检查批次规划" : ""}
      </div>
      {executionDurationMs !== undefined || schedulerWaitMs !== undefined ? (
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-base-content/65">
          <span>执行用时：{formatManagedTaskDuration(executionDurationMs)}</span>
          <span>调度等待：{formatManagedTaskDuration(schedulerWaitMs)}</span>
        </div>
      ) : null}
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
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 sm:grid-cols-3 xl:grid-cols-9">
              {[
                [
                  "已提交 / 批次",
                  `${batch.committedRows ?? "未知"} / ${batch.batchRows ?? "未知"} 条`,
                ],
                [
                  "服务速率",
                  rate(
                    serviceRate(
                      batch.committedRowsPerSecond,
                      batch.committedRows,
                      canUseRunFallback ? executionDurationMs : null,
                    ),
                  ),
                ],
                ["到达速率（24h）", rate(batch.arrivalRowsPerSecond)],
                [
                  "服务倍率",
                  (() => {
                    const observedServiceRate = serviceRate(
                      batch.committedRowsPerSecond,
                      batch.committedRows,
                      canUseRunFallback ? executionDurationMs : null,
                    );
                    const observedMultiple = serviceRateMultiple(
                      batch.serviceRateMultiple,
                      observedServiceRate,
                      batch.arrivalRowsPerSecond,
                    );
                    return observedMultiple == null ? "未知" : `${observedMultiple.toFixed(1)}×`;
                  })(),
                ],
                ["文件准备", duration(batch.filePrepareMs)],
                ["身份读取", duration(batch.sourceIdentityMs)],
                ["归档写入", duration(batch.archiveWriterMs)],
                ["证明准备", duration(batch.summaryProofMs)],
                ["SQLite 写入准入等待", duration(batch.lockWaitMs)],
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
    </section>
  );
}
