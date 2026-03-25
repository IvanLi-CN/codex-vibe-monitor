/* eslint-disable react-refresh/only-export-components */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { createPortal } from 'react-dom'
import { AppIcon } from '../AppIcon'
import { Button } from './button'
import type { MotherSwitchSnapshot } from '../../lib/upstreamMother'
import { useTranslation } from '../../i18n'
import { floatingSurfaceStyle } from './floating-surface'
import { usePortaledTheme } from './use-portaled-theme'

const DEFAULT_DURATION_MS = 10_000

interface MotherSwitchUndoNotification {
  id: string
  kind: 'motherSwitchUndo'
  groupKey: string
  payload: MotherSwitchSnapshot
  onUndo: () => Promise<void>
  error: string | null
}

interface ShowMotherSwitchUndoOptions {
  payload: MotherSwitchSnapshot
  onUndo: () => Promise<void>
}

interface SystemNotificationsContextValue {
  showMotherSwitchUndo: (options: ShowMotherSwitchUndoOptions) => void
  dismissNotification: (id: string) => void
}

const SystemNotificationsContext = createContext<SystemNotificationsContextValue | null>(null)

function useAutoDismiss(
  notificationId: string,
  pending: boolean,
  onDismiss: (id: string) => void,
) {
  const timerRef = useRef<number | null>(null)
  const startedAtRef = useRef(0)
  const remainingRef = useRef(DEFAULT_DURATION_MS)

  const clearTimer = useCallback(() => {
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
  }, [])

  const schedule = useCallback(() => {
    clearTimer()
    if (pending) return
    startedAtRef.current = Date.now()
    timerRef.current = window.setTimeout(() => {
      onDismiss(notificationId)
    }, remainingRef.current)
  }, [clearTimer, notificationId, onDismiss, pending])

  useEffect(() => {
    remainingRef.current = DEFAULT_DURATION_MS
    schedule()
    return clearTimer
  }, [clearTimer, notificationId, schedule])

  useEffect(() => {
    if (pending) {
      clearTimer()
      return
    }
    schedule()
  }, [clearTimer, pending, schedule])

  const pause = useCallback(() => {
    if (pending) return
    const elapsed = Date.now() - startedAtRef.current
    remainingRef.current = Math.max(0, remainingRef.current - elapsed)
    clearTimer()
  }, [clearTimer, pending])

  const resume = useCallback(() => {
    if (pending || remainingRef.current <= 0) return
    schedule()
  }, [pending, schedule])

  return { pause, resume }
}

function MotherSwitchUndoToast({
  notification,
  onDismiss,
  onUndoSettled,
  theme,
}: {
  notification: MotherSwitchUndoNotification
  onDismiss: (id: string) => void
  onUndoSettled: (id: string, error: string | null) => void
  theme: 'vibe-light' | 'vibe-dark' | undefined
}) {
  const { t } = useTranslation()
  const [pending, setPending] = useState(false)
  const { pause, resume } = useAutoDismiss(notification.id, pending, onDismiss)

  const groupLabel = notification.payload.groupName ?? t('accountPool.upstreamAccounts.groupFilter.ungrouped')
  const message = useMemo(() => {
    if (notification.payload.newMotherDisplayName && notification.payload.previousMotherDisplayName) {
      return t('accountPool.upstreamAccounts.mother.notifications.replaced', {
        group: groupLabel,
        previous: notification.payload.previousMotherDisplayName,
        next: notification.payload.newMotherDisplayName,
      })
    }
    if (notification.payload.newMotherDisplayName) {
      return t('accountPool.upstreamAccounts.mother.notifications.created', {
        group: groupLabel,
        next: notification.payload.newMotherDisplayName,
      })
    }
    return t('accountPool.upstreamAccounts.mother.notifications.cleared', {
      group: groupLabel,
      previous: notification.payload.previousMotherDisplayName ?? t('accountPool.upstreamAccounts.mother.badge'),
    })
  }, [groupLabel, notification.payload, t])

  const handleUndo = useCallback(async () => {
    setPending(true)
    try {
      await notification.onUndo()
      onDismiss(notification.id)
    } catch (error) {
      onUndoSettled(notification.id, error instanceof Error ? error.message : String(error))
    } finally {
      setPending(false)
    }
  }, [notification, onDismiss, onUndoSettled])

  return (
    <div
      data-theme={theme}
      role="status"
      aria-live="polite"
      style={floatingSurfaceStyle('warning', theme)}
      className="pointer-events-auto w-full max-w-md rounded-[1.4rem] border p-4 text-base-content"
      onMouseEnter={pause}
      onMouseLeave={resume}
    >
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-warning/16 text-warning">
          <AppIcon name="crown" className="h-5 w-5" aria-hidden />
        </div>
        <div className="min-w-0 flex-1 space-y-2">
          <div className="space-y-1">
            <p className="text-sm font-semibold">{t('accountPool.upstreamAccounts.mother.notifications.title')}</p>
            <p className="text-sm leading-6 text-base-content/82">{message}</p>
          </div>
          {notification.error ? <p className="text-xs text-error">{notification.error}</p> : null}
          <div className="flex items-center gap-2">
            <Button
              type="button"
              size="sm"
              variant="secondary"
              onClick={() => void handleUndo()}
              disabled={pending}
              className="h-8 rounded-full bg-warning/85 px-3 text-warning-content hover:bg-warning"
            >
              {pending ? (
                <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
              ) : (
                <AppIcon name="undo-variant" className="mr-2 h-4 w-4" aria-hidden />
              )}
              {t('accountPool.upstreamAccounts.mother.notifications.undo')}
            </Button>
            <button
              type="button"
              className="inline-flex h-8 items-center rounded-full px-3 text-xs font-medium text-base-content/72 transition hover:bg-white/10 hover:text-base-content"
              onClick={() => onDismiss(notification.id)}
            >
              {t('accountPool.upstreamAccounts.mother.notifications.dismiss')}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}

export function SystemNotificationProvider({ children }: { children: ReactNode }) {
  const [notifications, setNotifications] = useState<MotherSwitchUndoNotification[]>([])
  const portalTheme = usePortaledTheme(null)

  const dismissNotification = useCallback((id: string) => {
    setNotifications((current) => current.filter((item) => item.id !== id))
  }, [])

  const showMotherSwitchUndo = useCallback((options: ShowMotherSwitchUndoOptions) => {
    const nextNotification: MotherSwitchUndoNotification = {
      id: `mother-switch-${options.payload.groupKey || 'ungrouped'}-${Date.now()}`,
      kind: 'motherSwitchUndo',
      groupKey: options.payload.groupKey,
      payload: options.payload,
      onUndo: options.onUndo,
      error: null,
    }
    setNotifications((current) => {
      const filtered = current.filter(
        (item) => !(item.kind === 'motherSwitchUndo' && item.groupKey === options.payload.groupKey),
      )
      return [nextNotification, ...filtered]
    })
  }, [])

  const handleUndoSettled = useCallback((id: string, error: string | null) => {
    setNotifications((current) =>
      current.map((item) => (item.id === id ? { ...item, error } : item)),
    )
  }, [])

  const value = useMemo<SystemNotificationsContextValue>(
    () => ({
      showMotherSwitchUndo,
      dismissNotification,
    }),
    [dismissNotification, showMotherSwitchUndo],
  )

  return (
    <SystemNotificationsContext.Provider value={value}>
      {children}
      {typeof document === 'undefined'
        ? null
        : createPortal(
            // Intentional body-root overlay: global notifications must stay above every local host.
            <div className="pointer-events-none fixed inset-x-0 top-4 z-[120] mx-auto flex w-full max-w-5xl flex-col gap-3 px-4">
              {notifications.map((notification) => (
                <MotherSwitchUndoToast
                  key={notification.id}
                  notification={notification}
                  onDismiss={dismissNotification}
                  onUndoSettled={handleUndoSettled}
                  theme={portalTheme}
                />
              ))}
            </div>,
            document.body,
          )}
    </SystemNotificationsContext.Provider>
  )
}

export function useSystemNotifications() {
  const context = useContext(SystemNotificationsContext)
  if (!context) {
    throw new Error('useSystemNotifications must be used within a SystemNotificationProvider')
  }
  return context
}
