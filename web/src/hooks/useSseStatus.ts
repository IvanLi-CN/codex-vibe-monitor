import { useEffect, useState } from 'react'
import { getCurrentSseStatus, subscribeToSseStatus, type SseStatus } from '../lib/sse'

export default function useSseStatus() {
  const [status, setStatus] = useState<SseStatus>(() => getCurrentSseStatus())

  useEffect(() => {
    const unsubscribe = subscribeToSseStatus((next) => {
      setStatus(next)
    })
    return unsubscribe
  }, [])

  return status
}

