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
      const previousById = new Map(previousItems.map((item) => [item.id, item] as const))
      const nextById = new Map(nextItems.map((item) => [item.id, item] as const))
      const changes = detectMotherSwitches(previousItems, nextItems)
      const consumedGroupKeys = new Set<string>()

      for (const [accountId, previous] of previousById) {
        const next = nextById.get(accountId)
        if (!next) continue
        const previousGroup = normalizeMotherGroupKey(previous.groupName)
        const nextGroup = normalizeMotherGroupKey(next.groupName)
        if (previousGroup === nextGroup || (!previous.isMother && !next.isMother)) {
          continue
        }

        const relatedChanges = changes.filter(
          (change) =>
            change.previousMotherAccountId === accountId || change.newMotherAccountId === accountId,
        )
        if (relatedChanges.length === 0) continue

        const primaryChange =
          (next.isMother
            ? relatedChanges.find(
                (change) =>
                  change.groupKey === nextGroup && change.newMotherAccountId === accountId,
              )
            : relatedChanges.find(
                (change) =>
                  change.groupKey === previousGroup &&
                  change.previousMotherAccountId === accountId,
              )) ?? relatedChanges[0]

        relatedChanges.forEach((change) => consumedGroupKeys.add(change.groupKey))
        showMotherSwitchUndo({
          payload: primaryChange,
          onUndo: async () => {
            await updateUpstreamAccount(accountId, {
              groupName: previous.groupName?.trim() ?? '',
              isMother: previous.isMother,
            })

            if (
              next.isMother &&
              primaryChange.previousMotherAccountId != null &&
              primaryChange.previousMotherAccountId !== accountId
            ) {
              await updateUpstreamAccount(primaryChange.previousMotherAccountId, {
                isMother: true,
              })
            }

            emitUpstreamAccountsChanged()
          },
        })
      }

      for (const change of changes) {
        if (change.previousMotherAccountId == null && change.newMotherAccountId == null) {
          continue
        }
        if (consumedGroupKeys.has(change.groupKey)) {
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
