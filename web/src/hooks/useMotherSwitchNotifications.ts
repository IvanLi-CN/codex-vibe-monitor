import { useCallback } from 'react'
import { useSystemNotifications } from '../components/ui/system-notifications'
import type { UpstreamAccountSummary } from '../lib/api'
import { updateUpstreamAccount } from '../lib/api'
import { emitUpstreamAccountsChanged } from '../lib/upstreamAccountsEvents'
import { detectMotherSwitches } from '../lib/upstreamMother'

export function useMotherSwitchNotifications() {
  const { showMotherSwitchUndo } = useSystemNotifications()

  return useCallback(
    (previousItems: UpstreamAccountSummary[], nextItems: UpstreamAccountSummary[]) => {
      for (const change of detectMotherSwitches(previousItems, nextItems)) {
        if (change.previousMotherAccountId == null && change.newMotherAccountId == null) {
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
