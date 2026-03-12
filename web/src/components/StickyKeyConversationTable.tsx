import { useTranslation } from '../i18n'
import type { StickyKeyConversation, UpstreamStickyConversationsResponse } from '../lib/api'
import { KeyedConversationTable } from './KeyedConversationTable'

interface StickyKeyConversationTableProps {
  stats: UpstreamStickyConversationsResponse | null
  isLoading: boolean
  error?: string | null
}

export function StickyKeyConversationTable({ stats, isLoading, error }: StickyKeyConversationTableProps) {
  const { t } = useTranslation()

  return (
    <KeyedConversationTable<StickyKeyConversation>
      stats={stats}
      isLoading={isLoading}
      error={error}
      getConversationKey={(conversation) => conversation.stickyKey}
      keyColumnLabel={t('accountPool.upstreamAccounts.stickyConversations.table.stickyKey')}
      emptyLabel={t('accountPool.upstreamAccounts.stickyConversations.empty')}
      chartAriaLabel={t('accountPool.upstreamAccounts.stickyConversations.chartAria')}
    />
  )
}
