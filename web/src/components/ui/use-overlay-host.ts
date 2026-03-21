import * as React from 'react'

export const OverlayHostContext = React.createContext<HTMLElement | null>(null)

export function useResolvedOverlayContainer(container?: HTMLElement | null) {
  const inheritedHost = React.useContext(OverlayHostContext)
  return container !== undefined ? container : inheritedHost
}

export function useOverlayHostElement<T extends HTMLElement>(
  ref: React.Ref<T> | undefined,
) {
  const [hostElement, setHostElement] = React.useState<T | null>(null)

  const handleRef = React.useCallback(
    (node: T | null) => {
      setHostElement(node)
      if (typeof ref === 'function') {
        ref(node)
      } else if (ref) {
        ref.current = node
      }
    },
    [ref],
  )

  return {
    hostElement,
    ref: handleRef,
  }
}
