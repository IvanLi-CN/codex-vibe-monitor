import type {
  BulkUpstreamAccountSyncCounts,
  BulkUpstreamAccountSyncRow,
  BulkUpstreamAccountSyncSnapshot,
} from "../../lib/api";

export function bulkSyncRowStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "succeeded") return "success";
  if (status === "pending") return "warning";
  if (status === "failed") return "error";
  return "secondary";
}

function computeBulkSyncCounts(
  rows: BulkUpstreamAccountSyncRow[],
): BulkUpstreamAccountSyncCounts {
  return rows.reduce<BulkUpstreamAccountSyncCounts>(
    (counts, row) => {
      counts.total += 1;
      if (row.status === "succeeded") {
        counts.succeeded += 1;
        counts.completed += 1;
      } else if (row.status === "failed") {
        counts.failed += 1;
        counts.completed += 1;
      } else if (row.status === "skipped") {
        counts.skipped += 1;
        counts.completed += 1;
      }
      return counts;
    },
    {
      total: 0,
      completed: 0,
      succeeded: 0,
      failed: 0,
      skipped: 0,
    },
  );
}

export function resolveBulkSyncCounts(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  counts?: BulkUpstreamAccountSyncCounts | null,
): BulkUpstreamAccountSyncCounts {
  return counts ?? computeBulkSyncCounts(snapshot.rows);
}

export function withBulkSyncSnapshotStatus(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  status: BulkUpstreamAccountSyncSnapshot["status"],
): BulkUpstreamAccountSyncSnapshot {
  if (snapshot.status === status) return snapshot;
  return {
    ...snapshot,
    status,
  };
}

export function shouldAutoHideBulkSyncProgress(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  counts: BulkUpstreamAccountSyncCounts,
): boolean {
  return (
    snapshot.status === "completed" &&
    counts.failed === 0 &&
    counts.skipped === 0
  );
}
