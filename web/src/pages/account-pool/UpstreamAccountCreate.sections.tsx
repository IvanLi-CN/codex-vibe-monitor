import { AppIcon } from "../../components/AppIcon";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "../../components/ui/segmented-control";
import { Spinner } from "../../components/ui/spinner";
import { useUpstreamAccountCreateViewContext } from "./UpstreamAccountCreate.controller-context";
import { UpstreamAccountCreateDialogs } from "./UpstreamAccountCreate.dialogs";
import { UpstreamAccountCreatePrimaryCard } from "./UpstreamAccountCreate.primary-card";

const CREATE_TABS = ["oauth", "batchOauth", "import", "apiKey"] as const;

export function UpstreamAccountCreatePageSections() {
  const {
    actionError,
    activeTab,
    formatDateTime,
    handleTabChange,
    isLoading,
    isRelinking,
    listError,
    relinkSummary,
    session,
    sessionHint,
    t,
    writesEnabled,
  } = useUpstreamAccountCreateViewContext();

  return (
    <div className="grid gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="section-heading">
              <Button
                asChild
                variant="ghost"
                size="sm"
                className="mb-1 self-start px-0"
              >
                <Link to="/account-pool/upstream-accounts">
                  <AppIcon
                    name="arrow-left"
                    className="mr-2 h-4 w-4"
                    aria-hidden
                  />
                  {t("accountPool.upstreamAccounts.actions.backToList")}
                </Link>
              </Button>
              <h2 className="section-title">
                {isRelinking
                  ? t("accountPool.upstreamAccounts.createPage.relinkTitle")
                  : t("accountPool.upstreamAccounts.createPage.title")}
              </h2>
              <p className="section-description">
                {isRelinking
                  ? t(
                      "accountPool.upstreamAccounts.createPage.relinkDescription",
                      {
                        name:
                          relinkSummary?.displayName ??
                          t("accountPool.upstreamAccounts.unavailable"),
                      },
                    )
                  : t("accountPool.upstreamAccounts.createPage.description")}
              </p>
            </div>
            {isLoading ? <Spinner className="text-primary" /> : null}
          </div>

          {!writesEnabled ? (
            <Alert variant="warning">
              <AppIcon
                name="shield-key-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>
                <p className="font-medium">
                  {t("accountPool.upstreamAccounts.writesDisabledTitle")}
                </p>
                <p className="mt-1 text-sm text-warning/90">
                  {t("accountPool.upstreamAccounts.writesDisabledBody")}
                </p>
              </div>
            </Alert>
          ) : null}

          {listError || actionError ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>{actionError ?? listError}</div>
            </Alert>
          ) : null}

          {session ? (
            <Alert
              variant={
                session.status === "completed"
                  ? "success"
                  : session.status === "pending"
                    ? "info"
                    : "warning"
              }
            >
              <AppIcon
                name={
                  session.status === "completed"
                    ? "check-circle-outline"
                    : "link-variant-plus"
                }
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="space-y-1">
                <p className="font-medium">
                  {t(
                    `accountPool.upstreamAccounts.oauth.status.${session.status}`,
                  )}
                </p>
                <p className="text-sm opacity-90">
                  {sessionHint ??
                    session.error ??
                    formatDateTime(session.expiresAt)}
                </p>
              </div>
            </Alert>
          ) : sessionHint ? (
            <Alert variant="warning">
              <AppIcon
                name="refresh-circle"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="text-sm">{sessionHint}</div>
            </Alert>
          ) : null}

          {!isRelinking ? (
            <SegmentedControl
              className="self-start"
              role="tablist"
              aria-label={t("accountPool.upstreamAccounts.createPage.tabsLabel")}
            >
              {CREATE_TABS.map((tab) => (
                <SegmentedControlItem
                  key={tab}
                  active={activeTab === tab}
                  role="tab"
                  aria-selected={activeTab === tab}
                  onClick={() => handleTabChange(tab)}
                >
                  {tab === "oauth"
                    ? t("accountPool.upstreamAccounts.createPage.tabs.oauth")
                    : tab === "batchOauth"
                      ? t(
                          "accountPool.upstreamAccounts.createPage.tabs.batchOauth",
                        )
                      : tab === "import"
                        ? t(
                            "accountPool.upstreamAccounts.createPage.tabs.import",
                          )
                        : t(
                            "accountPool.upstreamAccounts.createPage.tabs.apiKey",
                          )}
                </SegmentedControlItem>
              ))}
            </SegmentedControl>
          ) : null}

          <UpstreamAccountCreatePrimaryCard />
        </div>
      </section>
      <UpstreamAccountCreateDialogs />
    </div>
  );
}
