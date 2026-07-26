import { useEffect, useMemo, useRef, useState } from "react";

export function useRoutingBlockClock(
  untilValues: Array<string | null | undefined>,
  onExpired?: () => void,
  enabled = true,
) {
  const [nowMs, setNowMs] = useState(() => Date.now());
  const expiryRefreshRef = useRef<string | null>(null);
  const expiryKey = useMemo(
    () =>
      untilValues
        .filter((until): until is string => Boolean(until))
        .sort()
        .join(","),
    [untilValues],
  );

  useEffect(() => {
    if (!enabled) return;
    const untils = expiryKey.split(",").filter(Boolean);
    if (untils.length === 0) return;
    const tick = () => {
      const currentNowMs = Date.now();
      setNowMs(currentNowMs);
      if (
        untils.some((until) => {
          const expiry = Date.parse(until);
          return Number.isFinite(expiry) && expiry <= currentNowMs;
        }) &&
        expiryRefreshRef.current !== expiryKey
      ) {
        expiryRefreshRef.current = expiryKey;
        onExpired?.();
      }
    };
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [enabled, expiryKey, onExpired]);

  return nowMs;
}
