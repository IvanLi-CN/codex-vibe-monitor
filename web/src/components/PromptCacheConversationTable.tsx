import { useTranslation } from '../i18n'
import type { PromptCacheConversation, PromptCacheConversationsResponse } from '../lib/api'
import { KeyedConversationTable } from './KeyedConversationTable'

interface PromptCacheConversationTableProps {
  stats: PromptCacheConversationsResponse | null
  isLoading: boolean
  error?: string | null
}

export function PromptCacheConversationTable({ stats, isLoading, error }: PromptCacheConversationTableProps) {
  const { t } = useTranslation()

  return (
    <KeyedConversationTable<PromptCacheConversation>
      stats={stats}
      isLoading={isLoading}
      error={error}
      getConversationKey={(conversation) => conversation.promptCacheKey}
      keyColumnLabel={t('live.conversations.table.promptCacheKey')}
      emptyLabel={t('live.conversations.empty')}
      chartAriaLabel={t('live.conversations.chartAria')}
    />
  )
}
