import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { VersionResponse } from "../lib/api";
import { buildTopicDescriptor, subscribeToTopic } from "../lib/sse";

const DISMISS_KEY = "update-dismissed-version";

export function useUpdateAvailable() {
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [availableVersion, setAvailableVersion] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  const initialVersionRef = useRef<string | null>(null);

  const dismissed = useMemo(() => {
    try {
      return localStorage.getItem(DISMISS_KEY);
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    const topic = buildTopicDescriptor("app.version");
    const unsubscribe = subscribeToTopic<VersionResponse>(topic, (event) => {
      const next = event.payload.backend ?? null;
      if (!next) return;
      const initial = initialVersionRef.current;
      if (!initial) {
        initialVersionRef.current = next;
        setCurrentVersion(next);
        return;
      }
      setCurrentVersion(next);
      if (next !== initial && next !== dismissed) {
        setAvailableVersion(next);
        setVisible(true);
      }
    });
    return unsubscribe;
  }, [dismissed]);

  const dismiss = useCallback(() => {
    if (availableVersion) {
      try {
        localStorage.setItem(DISMISS_KEY, availableVersion);
      } catch (err) {
        // ignore storage errors (Safari private mode, etc.)
        void err;
      }
    }
    setVisible(false);
  }, [availableVersion]);

  const reload = useCallback(() => {
    window.location.reload();
  }, []);

  // Dev-only helper to force showing the banner
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    (
      window as unknown as { __DEV_FORCE_UPDATE_BANNER__?: () => void }
    ).__DEV_FORCE_UPDATE_BANNER__ = () => {
      setAvailableVersion((v) => v ?? (currentVersion ? `${currentVersion}-dev` : "dev-next"));
      setVisible(true);
    };
  }, [currentVersion]);

  return {
    currentVersion,
    availableVersion,
    visible,
    dismiss,
    reload,
  };
}

export default useUpdateAvailable;
