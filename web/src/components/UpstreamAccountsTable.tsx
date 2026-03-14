import type { KeyboardEvent } from "react";
import { Icon } from "@iconify/react";
import { Badge } from "./ui/badge";
import type { UpstreamAccountSummary } from "../lib/api";
import { cn } from "../lib/utils";

interface UpstreamAccountsTableProps {
  items: UpstreamAccountSummary[];
  selectedId: number | null;
  onSelect: (accountId: number) => void;
  emptyTitle: string;
  emptyDescription: string;
  labels: {
    sync: string;
    never: string;
    group: string;
    primary: string;
    secondary: string;
    nextReset: string;
    oauth: string;
    apiKey: string;
    duplicate: string;
    status: (value: string) => string;
  };
}

function windowPercent(value?: number | null) {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.max(0, Math.min(value ?? 0, 100));
}

function formatDateTime(value?: string | null, fallback = "—") {
  if (!value) return fallback;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(date);
}

function kindLabel(
  item: UpstreamAccountSummary,
  labels: UpstreamAccountsTableProps["labels"],
) {
  return item.kind === "oauth_codex" ? labels.oauth : labels.apiKey;
}

function badgeVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "active") return "success";
  if (status === "syncing") return "warning";
  if (status === "error" || status === "needs_reauth") return "error";
  return "secondary";
}

function CompactMeter({
  percent,
  text,
  resetText,
  accentClassName,
}: {
  percent: number;
  text: string;
  resetText?: string;
  accentClassName?: string;
}) {
  return (
    <div className="min-w-[11rem]">
      <div className="h-2 overflow-hidden rounded-full bg-base-300/60">
        <div
          className={cn("h-full rounded-full bg-primary", accentClassName)}
          style={{ width: `${percent}%` }}
        />
      </div>
      <div className="mt-2 flex items-center justify-between gap-3 text-xs text-base-content/62">
        <span className="truncate">{text}</span>
        <span className="font-semibold text-base-content/72">
          {Math.round(percent)}%
        </span>
      </div>
      {resetText ? (
        <div className="mt-1 text-[11px] text-base-content/48">{resetText}</div>
      ) : null}
    </div>
  );
}

function handleRowKeyDown(
  event: KeyboardEvent<HTMLTableRowElement>,
  accountId: number,
  onSelect: (accountId: number) => void,
) {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    onSelect(accountId);
  }
}

export function UpstreamAccountsTable({
  items,
  selectedId,
  onSelect,
  emptyTitle,
  emptyDescription,
  labels,
}: UpstreamAccountsTableProps) {
  if (items.length === 0) {
    return (
      <div className="flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 py-10 text-center">
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Icon
            icon="mdi:server-network-outline"
            className="h-7 w-7"
            aria-hidden
          />
        </div>
        <h3 className="text-lg font-semibold text-base-content">
          {emptyTitle}
        </h3>
        <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
          {emptyDescription}
        </p>
      </div>
    );
  }

  return (
    <div className="overflow-hidden rounded-[1.35rem] border border-base-300/80 bg-base-100/72">
      <div className="overflow-x-auto">
        <table className="min-w-[940px] w-full border-collapse">
          <thead>
            <tr className="border-b border-base-300/80 bg-base-100/86 text-left">
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                Account
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                {labels.group}
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                Status
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                Type
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                Plan
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                {labels.sync}
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                {labels.primary}
              </th>
              <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                {labels.secondary}
              </th>
              <th className="w-12 px-4 py-3" aria-hidden />
            </tr>
          </thead>
          <tbody>
            {items.map((item, index) => {
              const primary = windowPercent(item.primaryWindow?.usedPercent);
              const secondary = windowPercent(
                item.secondaryWindow?.usedPercent,
              );
              const primaryResetText = item.primaryWindow?.resetsAt
                ? `${labels.nextReset} ${formatDateTime(item.primaryWindow.resetsAt)}`
                : undefined;
              const secondaryResetText = item.secondaryWindow?.resetsAt
                ? `${labels.nextReset} ${formatDateTime(item.secondaryWindow.resetsAt)}`
                : undefined;
              const selected = item.id === selectedId;
              return (
                <tr
                  key={item.id}
                  role="button"
                  tabIndex={0}
                  aria-pressed={selected}
                  onClick={() => onSelect(item.id)}
                  onKeyDown={(event) =>
                    handleRowKeyDown(event, item.id, onSelect)
                  }
                  className={cn(
                    "cursor-pointer border-b border-base-300/70 align-top outline-none transition-colors last:border-b-0 hover:bg-base-100/88 focus-visible:bg-base-100/88",
                    selected && "bg-primary/10",
                    index % 2 === 1 && !selected && "bg-base-100/32",
                  )}
                >
                  <td className="px-4 py-4">
                    <div className="flex items-center gap-2">
                      <span className="max-w-[18rem] truncate text-base font-semibold text-base-content">
                        {item.displayName}
                      </span>
                      {item.duplicateInfo ? (
                        <Badge variant="warning">{labels.duplicate}</Badge>
                      ) : null}
                      {!item.enabled ? (
                        <span className="rounded-full bg-base-300/70 px-2 py-0.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
                          Off
                        </span>
                      ) : null}
                    </div>
                  </td>
                  <td className="px-4 py-4">
                    <div className="max-w-[12rem] truncate text-sm text-base-content/72">
                      {item.groupName?.trim() || "—"}
                    </div>
                  </td>
                  <td className="px-4 py-4">
                    <Badge variant={badgeVariant(item.status)}>
                      {labels.status(item.status)}
                    </Badge>
                  </td>
                  <td className="px-4 py-4">
                    <Badge variant="secondary">{kindLabel(item, labels)}</Badge>
                  </td>
                  <td className="px-4 py-4 text-sm text-base-content/72">
                    {item.planType ?? "—"}
                  </td>
                  <td className="px-4 py-4 text-sm text-base-content/72">
                    {formatDateTime(item.lastSuccessfulSyncAt, labels.never)}
                  </td>
                  <td className="px-4 py-4">
                    <CompactMeter
                      percent={primary}
                      text={item.primaryWindow?.usedText ?? "—"}
                      resetText={primaryResetText}
                    />
                  </td>
                  <td className="px-4 py-4">
                    <CompactMeter
                      percent={secondary}
                      text={item.secondaryWindow?.usedText ?? "—"}
                      resetText={secondaryResetText}
                      accentClassName="bg-secondary"
                    />
                  </td>
                  <td className="px-4 py-4 text-right">
                    <Icon
                      icon={
                        selected
                          ? "mdi:chevron-right-circle"
                          : "mdi:chevron-right"
                      }
                      className={cn(
                        "h-5 w-5",
                        selected ? "text-primary" : "text-base-content/35",
                      )}
                      aria-hidden
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
