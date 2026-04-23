import { useCallback, useMemo } from "react";
import { Link } from "react-router-dom";
import { AppIcon } from "../../components/AppIcon";
import {
  AccountPoolGroupSummary,
  type AccountPoolGroupSummaryLabels,
} from "../../components/AccountPoolGroupSummary";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Spinner } from "../../components/ui/spinner";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import { useTranslation } from "../../i18n";
import {
  buildAccountPoolGroupSummaries,
  normalizeAccountPoolGroupName,
} from "../../lib/accountPoolGroups";
import { useUpstreamAccountGroupSettingsDialog } from "./useUpstreamAccountGroupSettingsDialog";

function buildPresetGroupFilter(groupName: string | null) {
  if (groupName == null) {
    return {
      mode: "ungrouped" as const,
      query: "",
    };
  }
  const normalizedGroupName = normalizeAccountPoolGroupName(groupName);
  return {
    mode: "exact" as const,
    query: normalizedGroupName ?? groupName,
  };
}

export default function GroupsPage() {
  const { t } = useTranslation();
  const accountListQuery = useMemo(() => ({ includeAll: true }), []);
  const {
    items,
    groups,
    forwardProxyNodes,
    writesEnabled,
    isLoading,
    listError,
    refresh,
    saveGroupNote,
  } = useUpstreamAccounts(accountListQuery, {
    fallbackToFirstItem: false,
  });

  const groupedPlanLabel = useCallback(
    (planType?: string | null) => {
      const normalized = planType?.trim().toLowerCase();
      if (!normalized) return null;
      switch (normalized) {
        case "free":
          return t("accountPool.upstreamAccounts.plan.free");
        case "pro":
          return t("accountPool.upstreamAccounts.plan.plus");
        case "team":
          return t("accountPool.upstreamAccounts.plan.team");
        case "enterprise":
          return t("accountPool.upstreamAccounts.plan.enterprise");
        default:
          return normalized;
      }
    },
    [t],
  );

  const groupSummaries = useMemo(
    () =>
      buildAccountPoolGroupSummaries({
        items,
        groups,
        forwardProxyNodes,
        ungroupedLabel: t("accountPool.upstreamAccounts.groupFilter.ungrouped"),
        groupedPlanLabel: (planType) =>
          planType === "api"
            ? t("accountPool.upstreamAccounts.grouped.apiBadge")
            : groupedPlanLabel(planType),
      }),
    [forwardProxyNodes, groupedPlanLabel, groups, items, t],
  );

  const namedGroups = useMemo(
    () => groupSummaries.filter((group) => group.groupName != null),
    [groupSummaries],
  );
  const ungroupedGroup =
    groupSummaries.find((group) => group.groupName == null) ?? null;

  const groupSummaryLabels = useMemo<AccountPoolGroupSummaryLabels>(
    () => ({
      count: (count) =>
        t("accountPool.upstreamAccounts.grouped.accountCount", { count }),
      concurrency: (value) =>
        t("accountPool.upstreamAccounts.grouped.concurrency", { value }),
      exclusiveNode: t("accountPool.upstreamAccounts.grouped.exclusiveNode"),
      noteLabel: t("accountPool.groups.noteLabel"),
      noteEmpty: t("accountPool.groups.noteEmpty"),
      proxiesLabel: t("accountPool.upstreamAccounts.grouped.proxiesLabel"),
      proxiesEmpty: t("accountPool.upstreamAccounts.grouped.proxiesEmpty"),
      settingsLabel: t("accountPool.upstreamAccounts.groupNotes.actions.edit"),
      upstream429Enabled: (count) =>
        t("accountPool.groups.upstream429Enabled", { count }),
      upstream429Disabled: t("accountPool.groups.upstream429Disabled"),
    }),
    [t],
  );

  const {
    openEditor: openGroupSettingsEditor,
    dialog: groupSettingsDialog,
  } = useUpstreamAccountGroupSettingsDialog({
    writesEnabled,
    resolveGroupState: useCallback(
      (groupName) => {
        const normalizedGroupName = normalizeAccountPoolGroupName(groupName);
        if (!normalizedGroupName) return null;
        const group =
          groupSummaries.find(
            (candidate) => candidate.groupName === normalizedGroupName,
          ) ?? null;
        return {
          groupName: normalizedGroupName,
          note: group?.note ?? "",
          existing: group != null,
          concurrencyLimit: group?.concurrencyLimit ?? 0,
          boundProxyKeys: group?.boundProxyKeys ?? [],
          nodeShuntEnabled: group?.nodeShuntEnabled ?? false,
          upstream429RetryEnabled: group?.upstream429RetryEnabled ?? false,
          upstream429MaxRetries: group?.upstream429MaxRetries ?? 0,
        };
      },
      [groupSummaries],
    ),
    saveGroupSettings: useCallback(
      async (groupName, payload) => {
        await saveGroupNote(groupName, payload);
      },
      [saveGroupNote],
    ),
  });

  const showEmptyState = !isLoading && !listError && groupSummaries.length === 0;

  return (
    <div className="grid gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="section-heading">
              <h2 className="section-title">{t("accountPool.groups.title")}</h2>
              <p className="section-description">
                {t("accountPool.groups.description")}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2 text-sm text-base-content/60">
              <span>{t("accountPool.groups.namedGroupsCount", { count: namedGroups.length })}</span>
              {ungroupedGroup ? (
                <span>
                  {t("accountPool.groups.ungroupedAccountsCount", {
                    count: ungroupedGroup.items.length,
                  })}
                </span>
              ) : null}
            </div>
          </div>

          {listError ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="flex flex-1 flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <span>{listError}</span>
                <Button type="button" variant="secondary" onClick={() => void refresh()}>
                  <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
                  {t("accountPool.groups.retry")}
                </Button>
              </div>
            </Alert>
          ) : null}

          {isLoading && groupSummaries.length === 0 ? (
            <div
              data-testid="account-pool-groups-loading"
              className="flex min-h-[14rem] flex-col items-center justify-center rounded-[1.4rem] border border-dashed border-base-300/80 bg-base-100/60 px-6 py-10 text-center"
            >
              <Spinner className="h-6 w-6 text-primary" />
              <p className="mt-4 text-base font-semibold text-base-content">
                {t("accountPool.groups.loadingTitle")}
              </p>
              <p className="mt-2 max-w-md text-sm leading-6 text-base-content/65">
                {t("accountPool.groups.loadingDescription")}
              </p>
            </div>
          ) : null}

          {showEmptyState ? (
            <div
              data-testid="account-pool-groups-empty"
              className="flex min-h-[14rem] flex-col items-center justify-center rounded-[1.4rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 py-10 text-center"
            >
              <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
                <AppIcon
                  name="badge-account-horizontal-outline"
                  className="h-7 w-7"
                  aria-hidden
                />
              </div>
              <p className="text-lg font-semibold text-base-content">
                {t("accountPool.groups.emptyTitle")}
              </p>
              <p className="mt-2 max-w-md text-sm leading-6 text-base-content/65">
                {t("accountPool.groups.emptyDescription")}
              </p>
              <Button asChild className="mt-5">
                <Link to="/account-pool/upstream-accounts/new">
                  <AppIcon name="plus" className="mr-2 h-4 w-4" aria-hidden />
                  {t("accountPool.groups.emptyCta")}
                </Link>
              </Button>
            </div>
          ) : null}

          {groupSummaries.length > 0 ? (
            <div
              data-testid="account-pool-groups-grid"
              className="grid gap-4 xl:grid-cols-2"
            >
              {namedGroups.map((group) => (
                <Card
                  key={group.id}
                  data-testid="account-pool-group-card"
                  className="border-base-300/70 bg-base-100/80 shadow-[0_12px_30px_rgba(15,23,42,0.07)]"
                >
                  <CardHeader className="pb-3">
                    <CardTitle className="text-base">
                      {t("accountPool.groups.cardTitle")}
                    </CardTitle>
                    <CardDescription>
                      {t("accountPool.groups.cardDescription")}
                    </CardDescription>
                  </CardHeader>
                  <CardContent className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_auto]">
                    <AccountPoolGroupSummary
                      group={group}
                      labels={groupSummaryLabels}
                      showNote
                      showRetryState
                    />
                    <div className="flex flex-col items-stretch gap-2 xl:justify-end">
                      <Button
                        type="button"
                        variant="secondary"
                        disabled={!writesEnabled}
                        onClick={() => {
                          if (!group.groupName) return;
                          openGroupSettingsEditor(group.groupName);
                        }}
                      >
                        <AppIcon
                          name="file-document-edit-outline"
                          className="mr-2 h-4 w-4"
                          aria-hidden
                        />
                        {t("accountPool.groups.editGroup")}
                      </Button>
                      <Button asChild>
                        <Link
                          to="/account-pool/upstream-accounts"
                          state={{
                            presetGroupFilter: buildPresetGroupFilter(
                              group.groupName,
                            ),
                          }}
                        >
                          <AppIcon
                            name="account-details-outline"
                            className="mr-2 h-4 w-4"
                            aria-hidden
                          />
                          {t("accountPool.groups.viewAccounts")}
                        </Link>
                      </Button>
                    </div>
                  </CardContent>
                </Card>
              ))}

              {ungroupedGroup ? (
                <Card
                  data-testid="account-pool-group-card-ungrouped"
                  className="border-dashed border-base-300/80 bg-base-100/70"
                >
                  <CardHeader className="pb-3">
                    <CardTitle className="text-base">
                      {t("accountPool.groups.ungroupedTitle")}
                    </CardTitle>
                    <CardDescription>
                      {t("accountPool.groups.ungroupedDescription")}
                    </CardDescription>
                  </CardHeader>
                  <CardContent className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_auto]">
                    <AccountPoolGroupSummary
                      group={ungroupedGroup}
                      labels={groupSummaryLabels}
                      showNote
                      showRetryState={false}
                    />
                    <div className="flex flex-col items-stretch gap-2 xl:justify-end">
                      <Button asChild>
                        <Link
                          to="/account-pool/upstream-accounts"
                          state={{
                            presetGroupFilter: buildPresetGroupFilter(null),
                          }}
                        >
                          <AppIcon
                            name="account-details-outline"
                            className="mr-2 h-4 w-4"
                            aria-hidden
                          />
                          {t("accountPool.groups.viewAccounts")}
                        </Link>
                      </Button>
                    </div>
                  </CardContent>
                </Card>
              ) : null}
            </div>
          ) : null}
        </div>
      </section>
      {groupSettingsDialog}
    </div>
  );
}
