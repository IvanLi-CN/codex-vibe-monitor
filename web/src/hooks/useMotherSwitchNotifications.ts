import { useCallback } from 'react'
import { useSystemNotifications } from '../components/ui/system-notifications'
import type { UpstreamAccountSummary } from '../lib/api'
import { updateUpstreamAccount } from '../lib/api'
import { emitUpstreamAccountsChanged } from '../lib/upstreamAccountsEvents'
import { detectMotherSwitches, normalizeMotherGroupKey } from '../lib/upstreamMother'

export function useMotherSwitchNotifications() {
  const { showMotherSwitchUndo } = useSystemNotifications()

  return useCallback(
    (previousItems: UpstreamAccountSummary[], nextItems: UpstreamAccountSummary[]) => {
      const movedMotherAccountIds = new Set<number>()
      const previousById = new Map(previousItems.map((item) => [item.id, item] as const))
      const nextById = new Map(nextItems.map((item) => [item.id, item] as const))

      for (const [accountId, previous] of previousById) {
        const next = nextById.get(accountId)
        if (!next) continue
        const previousGroup = normalizeMotherGroupKey(previous.groupName)
        const nextGroup = normalizeMotherGroupKey(next.groupName)
        if (previousGroup !== nextGroup && (previous.isMother || next.isMother)) {
          movedMotherAccountIds.add(accountId)
        }
      }

      for (const change of detectMotherSwitches(previousItems, nextItems)) {
        if (change.previousMotherAccountId == null && change.newMotherAccountId == null) {
          continue
        }
        if (
          (change.previousMotherAccountId != null
            && movedMotherAccountIds.has(change.previousMotherAccountId))
          || (change.newMotherAccountId != null
            && movedMotherAccountIds.has(change.newMotherAccountId))
        ) {
          continue
        }
        showMotherSwitchUndo({
          payload: change,
          onUndo: async () => {
            if (change.previousMotherAccountId != null) {
              await updateUpstreamAccount(change.previousMotherAccountId, { isMother: true })
            } else if (change.newMotherAccountId != null) {
              await updateUpstreamAccount(change.newMotherAccountId, { isMother: false })
            }
            emitUpstreamAccountsChanged()
          },
        })
      }
    },
    [showMotherSwitchUndo],
  )
}
