import * as React from 'react'
import { OverlayHostContext } from './use-overlay-host'

export function OverlayHostProvider({
  value,
  children,
}: {
  value?: HTMLElement | null
  children: React.ReactNode
}) {
  const parentHost = React.useContext(OverlayHostContext)
  return <OverlayHostContext.Provider value={value === undefined ? parentHost : value}>{children}</OverlayHostContext.Provider>
}
