import { useEffect, useMemo, useState } from "react";
import { AppIcon } from "./AppIcon";
import { Alert } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";
import { cn } from "../lib/utils";
import type { ImportedOauthValidationRow } from "../lib/api";
import { useTranslation } from "../i18n";

type ValidationFilterKey =
  | "pending"
  | "ok"
  | "exhausted"
  | "invalid"
  | "error"
  | "duplicate";

export type ImportedOauthValidationDialogState = {
  inputFiles: number;
  uniqueInInput: number;
  duplicateInInput: number;
  checking: boolean;
  importing: boolean;
  rows: ImportedOauthValidationRow[];
  importError?: string | null;
};

type ImportedOauthValidationDialogProps = {
  open: boolean;
  state: ImportedOauthValidationDialogState | null;
  importDisabledReason?: string | null;
  onClose: () => void;
  onRetryFailed: () => void;
  onRetryOne: (sourceId: string) => void;
  onImportValid: () => void;
};

type ValidationCounts = {
  pending: number;
  duplicate: number;
  ok: number;
  exhausted: number;
  invalid: number;
  error: number;
  checked: number;
};

function computeValidationCounts(
  state: ImportedOauthValidationDialogState | null,
): ValidationCounts {
  const counts: ValidationCounts = {
    pending: 0,
    duplicate: 0,
    ok: 0,
    exhausted: 0,
    invalid: 0,
    error: 0,
    checked: 0,
  };

  for (const row of state?.rows ?? []) {
    switch (row.status) {
      case "pending":
        counts.pending += 1;
        break;
      case "duplicate_in_input":
        counts.duplicate += 1;
        break;
      case "ok":
        counts.ok += 1;
        break;
      case "ok_exhausted":
        counts.exhausted += 1;
        break;
      case "invalid":
        counts.invalid += 1;
        break;
      case "error":
      default:
        counts.error += 1;
        break;
    }
  }

  counts.checked =
    counts.duplicate +
    counts.ok +
    counts.exhausted +
    counts.invalid +
    counts.error;
  return counts;
}

function filterKeyForStatus(
  status: ImportedOauthValidationRow["status"],
): ValidationFilterKey {
  switch (status) {
    case "pending":
      return "pending";
    case "duplicate_in_input":
      return "duplicate";
    case "ok":
      return "ok";
    case "ok_exhausted":
      return "exhausted";
    case "invalid":
      return "invalid";
    case "error":
    default:
      return "error";
  }
}

function rowBadgeVariant(status: ImportedOauthValidationRow["status"]) {
  switch (status) {
    case "ok":
      return "success" as const;
    case "ok_exhausted":
      return "warning" as const;
    case "pending":
      return "info" as const;
    case "duplicate_in_input":
      return "secondary" as const;
    case "invalid":
    case "error":
    default:
      return "error" as const;
  }
}

function rowAccentClass(status: ImportedOauthValidationRow["status"]) {
  switch (status) {
    case "ok":
      return "before:bg-success";
    case "ok_exhausted":
      return "before:bg-warning";
    case "pending":
      return "before:bg-info";
    case "duplicate_in_input":
      return "before:bg-base-content/30";
    case "invalid":
    case "error":
    default:
      return "before:bg-error";
  }
}

function rowSurfaceClass(status: ImportedOauthValidationRow["status"]) {
  switch (status) {
    case "ok":
      return "bg-success/10";
    case "ok_exhausted":
      return "bg-warning/10";
    case "pending":
      return "bg-info/10";
    case "duplicate_in_input":
      return "bg-base-200/40";
    case "invalid":
    case "error":
    default:
      return "bg-error/10";
  }
}

function formatStatusLabel(
  t: (key: string, values?: Record<string, string | number>) => string,
  status: ImportedOauthValidationRow["status"],
) {
  switch (status) {
    case "pending":
      return t("accountPool.upstreamAccounts.import.validation.status.pending");
    case "duplicate_in_input":
      return t(
        "accountPool.upstreamAccounts.import.validation.status.duplicate",
      );
    case "ok":
      return t("accountPool.upstreamAccounts.import.validation.status.ok");
    case "ok_exhausted":
      return t(
        "accountPool.upstreamAccounts.import.validation.status.exhausted",
      );
    case "invalid":
      return t("accountPool.upstreamAccounts.import.validation.status.invalid");
    case "error":
    default:
      return t("accountPool.upstreamAccounts.import.validation.status.error");
  }
}

function formatFilterLabel(
  t: (key: string, values?: Record<string, string | number>) => string,
  filter: ValidationFilterKey,
) {
  return formatStatusLabel(
    t,
    filter === "duplicate"
      ? "duplicate_in_input"
      : filter === "exhausted"
        ? "ok_exhausted"
        : filter,
  );
}

function FilterChip({
  active,
  label,
  count,
  share,
  onClick,
}: {
  active: boolean;
  label: string;
  count: number;
  share?: number | null;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "group w-full rounded-2xl border px-3 py-2 text-left transition-colors duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
        active
          ? "border-primary/35 bg-primary/10 text-primary"
          : "border-base-300/70 bg-base-100/85 text-base-content hover:border-base-300 hover:bg-base-100",
      )}
    >
      <div className="flex min-h-[5.5rem] items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col justify-center">
          <span className="text-sm font-medium">{label}</span>
          {share != null ? (
            <div className="mt-1 text-[11px] uppercase tracking-[0.08em] opacity-70">
              {share}%
            </div>
          ) : null}
        </div>
        <span className="font-mono text-[2rem] font-semibold leading-none">
          {count}
        </span>
      </div>
    </button>
  );
}

function InlineIdentityList({
  email,
  accountId,
  displayName,
}: {
  email?: string | null;
  accountId?: string | null;
  displayName?: string | null;
}) {
  const { t } = useTranslation();

  return (
    <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-base-content/65">
      <span className="truncate">
        {t("accountPool.upstreamAccounts.fields.email")}: {email || "—"}
      </span>
      <span className="truncate">
        {t("accountPool.upstreamAccounts.fields.accountId")}: {accountId || "—"}
      </span>
      <span className="truncate">
        {t("accountPool.upstreamAccounts.fields.displayName")}:{" "}
        {displayName || "—"}
      </span>
    </div>
  );
}

export const IMPORT_VALIDATION_PAGE_SIZE = 100;

export function ImportedOauthValidationDialog({
  open,
  state,
  importDisabledReason,
  onClose,
  onRetryFailed,
  onRetryOne,
  onImportValid,
}: ImportedOauthValidationDialogProps) {
  const { t } = useTranslation();
  const [activeFilter, setActiveFilter] = useState<ValidationFilterKey | null>(
    null,
  );
  const [page, setPage] = useState(1);
  const counts = useMemo(() => computeValidationCounts(state), [state]);
  const validRows = useMemo(
    () =>
      (state?.rows ?? []).filter(
        (row) => row.status === "ok" || row.status === "ok_exhausted",
      ),
    [state],
  );
  const filteredRows = useMemo(() => {
    const rows = state?.rows ?? [];
    if (!activeFilter) return rows;
    return rows.filter(
      (row) => filterKeyForStatus(row.status) === activeFilter,
    );
  }, [activeFilter, state]);
  const isBusy = state?.checking === true || state?.importing === true;
  const canRetryFailed = !isBusy && (counts.invalid > 0 || counts.error > 0);
  const canImportValid =
    !isBusy && validRows.length > 0 && !importDisabledReason;
  const totalSegments = Math.max(1, state?.uniqueInInput ?? 0);
  const progressSegments: Array<{
    key: ValidationFilterKey;
    count: number;
    tone: string;
  }> = [
    { key: "pending", count: counts.pending, tone: "bg-info" },
    { key: "ok", count: counts.ok, tone: "bg-success" },
    { key: "exhausted", count: counts.exhausted, tone: "bg-warning" },
    { key: "invalid", count: counts.invalid, tone: "bg-error" },
    { key: "error", count: counts.error, tone: "bg-error/65" },
  ];
  const filterItems: Array<{
    key: ValidationFilterKey;
    count: number;
    share: number | null;
  }> = [
    {
      key: "pending",
      count: counts.pending,
      share: Math.round((counts.pending / totalSegments) * 100),
    },
    {
      key: "ok",
      count: counts.ok,
      share: Math.round((counts.ok / totalSegments) * 100),
    },
    {
      key: "exhausted",
      count: counts.exhausted,
      share: Math.round((counts.exhausted / totalSegments) * 100),
    },
    {
      key: "invalid",
      count: counts.invalid,
      share: Math.round((counts.invalid / totalSegments) * 100),
    },
    {
      key: "error",
      count: counts.error,
      share: Math.round((counts.error / totalSegments) * 100),
    },
    {
      key: "duplicate",
      count: counts.duplicate,
      share: Math.round((counts.duplicate / totalSegments) * 100),
    },
  ];
  const totalPages = Math.max(
    1,
    Math.ceil(filteredRows.length / IMPORT_VALIDATION_PAGE_SIZE),
  );
  const pagedRows = useMemo(() => {
    const startIndex = (page - 1) * IMPORT_VALIDATION_PAGE_SIZE;
    return filteredRows.slice(
      startIndex,
      startIndex + IMPORT_VALIDATION_PAGE_SIZE,
    );
  }, [filteredRows, page]);

  useEffect(() => {
    setPage(1);
  }, [activeFilter, state?.rows.length]);

  useEffect(() => {
    if (page > totalPages) {
      setPage(totalPages);
    }
  }, [page, totalPages]);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen: boolean) => (!nextOpen ? onClose() : undefined)}
    >
      <DialogContent className="h-[min(92vh,56rem)] w-[min(96vw,92rem)] max-w-none overflow-hidden p-0">
        <div className="grid h-full min-h-0 grid-rows-[auto,minmax(0,1fr),auto]">
          <DialogHeader className="border-b border-base-300 bg-[linear-gradient(180deg,rgba(15,23,42,0.04),transparent)] px-6 pb-5 pt-5">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0">
                <DialogTitle>
                  {t("accountPool.upstreamAccounts.import.validation.title")}
                </DialogTitle>
                <DialogDescription className="mt-1 max-w-3xl text-sm leading-6 text-base-content/70">
                  {t(
                    "accountPool.upstreamAccounts.import.validation.description",
                    {
                      checked: counts.checked,
                      total: state?.uniqueInInput ?? 0,
                      files: state?.inputFiles ?? 0,
                    },
                  )}
                </DialogDescription>
              </div>
              <DialogCloseIcon />
            </div>

            <div className="mt-4 border-t border-base-300/65 pt-4">
              <div className="overflow-hidden rounded-full bg-base-200/90">
                <div className="flex h-2.5 w-full">
                  {progressSegments.map((segment) =>
                    segment.count > 0 ? (
                      <span
                        key={segment.key}
                        className={cn("h-full", segment.tone)}
                        style={{
                          width: `${(segment.count / totalSegments) * 100}%`,
                        }}
                      />
                    ) : null,
                  )}
                </div>
              </div>

              <div className="mt-4 grid gap-2 [grid-template-columns:repeat(auto-fit,minmax(10rem,1fr))]">
                {filterItems.map((item) => (
                  <FilterChip
                    key={item.key}
                    active={activeFilter === item.key}
                    label={formatFilterLabel(t, item.key)}
                    count={item.count}
                    share={item.share}
                    onClick={() =>
                      setActiveFilter((current) =>
                        current === item.key ? null : item.key,
                      )
                    }
                  />
                ))}
              </div>
            </div>
          </DialogHeader>

          <div className="grid h-full min-h-0 grid-rows-[auto,minmax(0,1fr)] overflow-hidden px-6 py-5">
            {state?.importError || importDisabledReason ? (
              <Alert variant="error" className="mb-4">
                <AppIcon
                  name="alert-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <div className="text-sm">
                  {state?.importError ?? importDisabledReason}
                </div>
              </Alert>
            ) : null}

            <section className="grid h-full min-h-0 grid-rows-[auto,minmax(0,1fr),auto] border-t border-base-300/65 pt-5">
              <div className="flex flex-wrap items-center justify-between gap-3 px-1 pb-3">
                <div>
                  <h3 className="text-sm font-semibold text-base-content">
                    {t(
                      "accountPool.upstreamAccounts.import.validation.resultsTitle",
                    )}
                  </h3>
                  <p className="mt-1 text-sm text-base-content/65">
                    {activeFilter
                      ? `${formatFilterLabel(t, activeFilter)} · ${t(
                          "accountPool.upstreamAccounts.import.validation.resultsCount",
                          {
                            shown: pagedRows.length,
                            total: state?.rows.length ?? 0,
                          },
                        )}`
                      : t(
                          "accountPool.upstreamAccounts.import.validation.resultsCount",
                          {
                            shown: pagedRows.length,
                            total: state?.rows.length ?? 0,
                          },
                        )}
                  </p>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  {totalPages > 1 ? (
                    <div className="text-sm text-base-content/70">
                      {t("records.list.pageLabel", { page, totalPages })}
                    </div>
                  ) : null}
                  {activeFilter ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      onClick={() => setActiveFilter(null)}
                    >
                      {t(
                        "accountPool.upstreamAccounts.import.validation.clearFilter",
                      )}
                    </Button>
                  ) : null}
                </div>
              </div>

              {filteredRows.length === 0 ? (
                <div className="flex min-h-0 items-center justify-center px-4 py-10 text-center text-sm text-base-content/65">
                  {state?.checking
                    ? t(
                        "accountPool.upstreamAccounts.import.validation.checking",
                      )
                    : t("accountPool.upstreamAccounts.import.validation.empty")}
                </div>
              ) : (
                <>
                  <div className="hidden min-h-0 overflow-hidden md:block">
                    <div className="h-full overflow-y-auto rounded-[1rem] border border-base-300/65">
                      <table className="min-w-full table-fixed text-sm">
                        <thead className="sticky top-0 z-10 bg-base-200/95 text-[11px] uppercase tracking-[0.08em] text-base-content/60 backdrop-blur">
                          <tr>
                            <th className="w-[32%] px-4 py-3 text-left font-semibold">
                              {t(
                                "accountPool.upstreamAccounts.import.validation.columns.file",
                              )}
                            </th>
                            <th className="w-[24%] px-4 py-3 text-left font-semibold">
                              {t(
                                "accountPool.upstreamAccounts.import.validation.columns.result",
                              )}
                            </th>
                            <th className="w-[30%] px-4 py-3 text-left font-semibold">
                              {t(
                                "accountPool.upstreamAccounts.import.validation.columns.detail",
                              )}
                            </th>
                            <th className="w-[14%] px-4 py-3 text-left font-semibold">
                              {t(
                                "accountPool.upstreamAccounts.import.validation.columns.actions",
                              )}
                            </th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-base-300/65">
                          {pagedRows.map((row) => {
                            const canRetryOne =
                              !isBusy &&
                              (row.status === "invalid" ||
                                row.status === "error");
                            return (
                              <tr
                                key={row.sourceId}
                                className={cn(
                                  "align-top",
                                  rowSurfaceClass(row.status),
                                )}
                              >
                                <td className="px-4 py-4">
                                  <div
                                    className={cn(
                                      "relative pl-4 before:absolute before:bottom-0 before:left-0 before:top-0 before:w-1 before:rounded-full",
                                      rowAccentClass(row.status),
                                    )}
                                  >
                                    <p className="truncate font-semibold text-base-content">
                                      {row.fileName}
                                    </p>
                                    <InlineIdentityList
                                      email={row.email}
                                      accountId={row.chatgptAccountId}
                                      displayName={row.displayName}
                                    />
                                  </div>
                                </td>
                                <td className="px-4 py-4">
                                  <div className="flex flex-wrap items-center gap-2">
                                    <Badge
                                      variant={rowBadgeVariant(row.status)}
                                    >
                                      {formatStatusLabel(t, row.status)}
                                    </Badge>
                                    {row.matchedAccount ? (
                                      <Badge variant="secondary">
                                        {t(
                                          "accountPool.upstreamAccounts.import.validation.matchedAccount",
                                          {
                                            name: row.matchedAccount
                                              .displayName,
                                          },
                                        )}
                                      </Badge>
                                    ) : null}
                                  </div>
                                  <div className="mt-2 text-xs text-base-content/65">
                                    {t(
                                      "accountPool.upstreamAccounts.fields.tokenExpiresAt",
                                    )}
                                    : {row.tokenExpiresAt || "—"}
                                  </div>
                                </td>
                                <td className="px-4 py-4">
                                  <p className="text-sm leading-6 text-base-content/75">
                                    {row.detail ||
                                      t(
                                        "accountPool.upstreamAccounts.import.validation.noDetail",
                                      )}
                                  </p>
                                  {row.attempts > 0 ? (
                                    <div className="mt-2">
                                      <Badge
                                        variant="secondary"
                                        className="font-mono"
                                      >
                                        {t(
                                          "accountPool.upstreamAccounts.import.validation.attempts",
                                          {
                                            count: row.attempts,
                                          },
                                        )}
                                      </Badge>
                                    </div>
                                  ) : null}
                                </td>
                                <td className="px-4 py-4">
                                  {canRetryOne ? (
                                    <Button
                                      type="button"
                                      variant="outline"
                                      size="sm"
                                      className="w-full justify-center"
                                      onClick={() => onRetryOne(row.sourceId)}
                                    >
                                      <AppIcon
                                        name="refresh"
                                        className="mr-2 h-4 w-4"
                                        aria-hidden
                                      />
                                      {t(
                                        "accountPool.upstreamAccounts.import.validation.retryOne",
                                      )}
                                    </Button>
                                  ) : (
                                    <span className="text-sm text-base-content/45">
                                      —
                                    </span>
                                  )}
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    </div>
                  </div>

                  <div className="min-h-0 overflow-y-auto md:hidden">
                    <div className="divide-y divide-base-300/65 overflow-hidden rounded-[1rem] border border-base-300/65">
                      {pagedRows.map((row) => {
                        const canRetryOne =
                          !isBusy &&
                          (row.status === "invalid" || row.status === "error");
                        return (
                          <div
                            key={`mobile-${row.sourceId}`}
                            className={cn(
                              "px-4 py-4",
                              rowSurfaceClass(row.status),
                            )}
                          >
                            <div
                              className={cn(
                                "relative pl-4 before:absolute before:bottom-0 before:left-0 before:top-0 before:w-1 before:rounded-full",
                                rowAccentClass(row.status),
                              )}
                            >
                              <div className="flex flex-wrap items-center gap-2">
                                <p className="min-w-0 flex-1 truncate font-semibold text-base-content">
                                  {row.fileName}
                                </p>
                                <Badge variant={rowBadgeVariant(row.status)}>
                                  {formatStatusLabel(t, row.status)}
                                </Badge>
                              </div>
                              <InlineIdentityList
                                email={row.email}
                                accountId={row.chatgptAccountId}
                                displayName={row.displayName}
                              />
                              <div className="mt-3 flex flex-wrap items-center gap-2">
                                {row.matchedAccount ? (
                                  <Badge variant="secondary">
                                    {t(
                                      "accountPool.upstreamAccounts.import.validation.matchedAccount",
                                      {
                                        name: row.matchedAccount.displayName,
                                      },
                                    )}
                                  </Badge>
                                ) : null}
                                {row.attempts > 0 ? (
                                  <Badge
                                    variant="secondary"
                                    className="font-mono"
                                  >
                                    {t(
                                      "accountPool.upstreamAccounts.import.validation.attempts",
                                      {
                                        count: row.attempts,
                                      },
                                    )}
                                  </Badge>
                                ) : null}
                              </div>
                              <p className="mt-3 text-sm leading-6 text-base-content/75">
                                {row.detail ||
                                  t(
                                    "accountPool.upstreamAccounts.import.validation.noDetail",
                                  )}
                              </p>
                              <div className="mt-2 text-xs text-base-content/65">
                                {t(
                                  "accountPool.upstreamAccounts.fields.tokenExpiresAt",
                                )}
                                : {row.tokenExpiresAt || "—"}
                              </div>
                              {canRetryOne ? (
                                <Button
                                  type="button"
                                  variant="outline"
                                  size="sm"
                                  className="mt-4 w-full justify-center"
                                  onClick={() => onRetryOne(row.sourceId)}
                                >
                                  <AppIcon
                                    name="refresh"
                                    className="mr-2 h-4 w-4"
                                    aria-hidden
                                  />
                                  {t(
                                    "accountPool.upstreamAccounts.import.validation.retryOne",
                                  )}
                                </Button>
                              ) : null}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                </>
              )}

              {totalPages > 1 ? (
                <div className="flex flex-wrap items-center justify-end gap-2 border-t border-base-300/65 px-1 pt-4">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      setPage((current) => Math.max(1, current - 1))
                    }
                    disabled={page <= 1}
                  >
                    {t("records.list.prev")}
                  </Button>
                  <div className="text-sm text-base-content/70">
                    {t("records.list.pageLabel", { page, totalPages })}
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() =>
                      setPage((current) => Math.min(totalPages, current + 1))
                    }
                    disabled={page >= totalPages}
                  >
                    {t("records.list.next")}
                  </Button>
                </div>
              ) : null}
            </section>
          </div>

          <DialogFooter className="border-t border-base-300 px-6 py-4">
            <div className="flex w-full flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
              <div className="flex flex-wrap items-center gap-2 text-sm text-base-content/65">
                <span>
                  {t(
                    "accountPool.upstreamAccounts.import.validation.footerHint",
                    {
                      valid: validRows.length,
                      duplicates: state?.duplicateInInput ?? 0,
                    },
                  )}
                </span>
                {counts.exhausted > 0 ? (
                  <Badge variant="warning">
                    {t(
                      "accountPool.upstreamAccounts.import.validation.status.exhausted",
                    )}{" "}
                    {counts.exhausted}
                  </Badge>
                ) : null}
              </div>
              <div className="flex flex-wrap justify-end gap-2">
                <Button type="button" variant="ghost" onClick={onClose}>
                  {t("accountPool.upstreamAccounts.actions.cancel")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={onRetryFailed}
                  disabled={!canRetryFailed}
                >
                  <AppIcon
                    name="refresh"
                    className="mr-2 h-4 w-4"
                    aria-hidden
                  />
                  {t(
                    "accountPool.upstreamAccounts.import.validation.retryFailed",
                  )}
                </Button>
                <Button
                  type="button"
                  onClick={onImportValid}
                  disabled={!canImportValid}
                >
                  {state?.importing ? (
                    <SpinnerInline />
                  ) : (
                    <AppIcon
                      name="content-save-plus-outline"
                      className="mr-2 h-4 w-4"
                      aria-hidden
                    />
                  )}
                  {t(
                    "accountPool.upstreamAccounts.import.validation.importValid",
                    { count: validRows.length },
                  )}
                </Button>
              </div>
            </div>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SpinnerInline() {
  return (
    <span className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-primary-content/35 border-t-primary-content" />
  );
}
