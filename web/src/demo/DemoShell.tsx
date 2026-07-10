import { useEffect, useSyncExternalStore, type ReactNode } from 'react'
import { useLocation } from 'react-router-dom'
import { useTheme } from '../theme'
import { demoModel } from './model'
import { sceneFromLocation, themeFromLocation } from './runtime'
import { DemoInspector } from './DemoInspector'

export function DemoShell({ children }: { children: ReactNode }) {
  const location = useLocation()
  const { setThemeMode } = useTheme()
  const snapshot = useSyncExternalStore(
    (listener) => demoModel.subscribe(listener),
    () => demoModel.snapshot,
    () => demoModel.snapshot,
  )

  useEffect(() => {
    demoModel.setScene(sceneFromLocation())
    setThemeMode(themeFromLocation())
  }, [location, setThemeMode])

  return (
    <>
      <div key={snapshot.scene}>{children}</div>
      <DemoInspector />
    </>
  )
}
