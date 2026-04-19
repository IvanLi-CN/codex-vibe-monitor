import { useId, useMemo } from "react";
import { useTranslation } from "../i18n";
import { FALLBACK_CELL } from "./invocation-details-shared";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import {
  type PromptCacheConversationHistoryQueryBuilder,
  type PromptCacheConversationHistoryRecordMatcher,
  PromptCacheConversationInvocationTable,
  usePromptCacheConversationHistory,
} from "./prompt-cache-conversation-history-shared";

interface PromptCacheConversationHistoryDrawerProps {
  open: boolean;
  conversationKey: string | null;
  onClose: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  historyQueryForConversationKey?: PromptCacheConversationHistoryQueryBuilder;
  historyRecordMatchesConversationKey?: PromptCacheConversationHistoryRecordMatcher;
}

export function PromptCacheConversationHistoryDrawer({
  open,
  conversationKey,
  onClose,
  onOpenUpstreamAccount,
  historyQueryForConversationKey,
  historyRecordMatchesConversationKey,
}: PromptCacheConversationHistoryDrawerProps) {
  const { t } = useTranslation();
  const titleId = useId();
  const {
    visibleRecords,
    effectiveTotal,
    loadedCount,
    isLoading,
    error,
    hasHydrated,
  } = usePromptCacheConversationHistory({
    open,
    conversationKey,
    historyQueryForConversationKey,
    historyRecordMatchesConversationKey,
  });

  const progressLabel = useMemo(() => {
    if (loadedCount < effectiveTotal) {
      return t("live.conversations.drawer.progress", {
        loaded: loadedCount,
        total: effectiveTotal,
      });
    }
    return t("live.conversations.drawer.progressComplete", {
      count: effectiveTotal,
    });
  }, [effectiveTotal, loadedCount, t]);
  const tableIsLoading =
    isLoading ||
    (open &&
      conversationKey != null &&
      !hasHydrated &&
      visibleRecords.length === 0 &&
      error == null);

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t("live.conversations.drawer.close")}
      onClose={onClose}
      header={
        <div className="space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary/70">
            {t("live.conversations.drawer.eyebrow")}
          </p>
          <div className="space-y-1">
            <h2 id={titleId} className="text-xl font-semibold text-base-content">
              {conversationKey?.trim() || FALLBACK_CELL}
            </h2>
            <p className="text-sm leading-6 text-base-content/70">
              {t("live.conversations.drawer.description")}
            </p>
          </div>
        </div>
      }
    >
      <div
        data-testid="prompt-cache-conversation-history-drawer"
        className="space-y-4"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
          <span className="text-base-content/70">{progressLabel}</span>
          {loadedCount > 0 && isLoading ? (
            <span className="text-xs text-base-content/58">
              {t("live.conversations.drawer.loadingMore")}
            </span>
          ) : null}
        </div>

        <PromptCacheConversationInvocationTable
          records={visibleRecords}
          isLoading={tableIsLoading}
          error={error}
          emptyLabel={t("live.conversations.drawer.empty")}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
        />
      </div>
    </AccountDetailDrawerShell>
  );
}
