import { registerSW } from "virtual:pwa-register";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { frontendVersion, normalizeVersion } from "../lib/version";

export type PwaInstallMode = "prompt" | "manual-ios" | "installed" | "unsupported";

interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed"; platform: string }>;
}

interface PwaRuntimeVersionState {
  currentVersion: string;
  availableVersion: string | null;
  visible: boolean;
}

export interface PwaRuntimeState {
  installMode: PwaInstallMode;
  installSupported: boolean;
  isOffline: boolean;
  shellReady: boolean;
  update: PwaRuntimeVersionState;
  promptInstall: () => Promise<void>;
  applyUpdate: () => Promise<void>;
  dismissUpdate: () => void;
}

const UPDATE_CHECK_INTERVAL_MS = 15 * 60 * 1000;

function isIosPlatform(
  nav: Navigator | undefined = typeof navigator === "undefined" ? undefined : navigator,
) {
  if (!nav) return false;
  const userAgent = nav.userAgent ?? "";
  const platform = nav.platform ?? "";
  const maxTouchPoints = typeof nav.maxTouchPoints === "number" ? nav.maxTouchPoints : 0;
  return /iPad|iPhone|iPod/.test(userAgent) || (platform === "MacIntel" && maxTouchPoints > 1);
}

function isSafariBrowser(
  nav: Navigator | undefined = typeof navigator === "undefined" ? undefined : navigator,
) {
  if (!nav) return false;
  const userAgent = nav.userAgent ?? "";
  return /Safari/.test(userAgent) && !/CriOS|FxiOS|EdgiOS|OPiOS|DuckDuckGo/.test(userAgent);
}

function isStandaloneDisplay(
  win: Window | undefined = typeof window === "undefined" ? undefined : window,
) {
  if (!win) return false;
  return (
    win.matchMedia("(display-mode: standalone)").matches ||
    win.matchMedia("(display-mode: window-controls-overlay)").matches ||
    (win.navigator as Navigator & { standalone?: boolean }).standalone === true
  );
}

function resolveInstallMode(promptEvent: BeforeInstallPromptEvent | null): PwaInstallMode {
  if (isStandaloneDisplay()) return "installed";
  if (promptEvent) return "prompt";
  if (isIosPlatform() && isSafariBrowser()) return "manual-ios";
  return "unsupported";
}

async function fetchFrontendVersion(): Promise<string | null> {
  const response = await fetch(`${import.meta.env.BASE_URL}version.json?ts=${Date.now()}`, {
    cache: "no-store",
  });
  if (!response.ok) return null;
  const payload = (await response.json()) as { version?: string };
  return normalizeVersion(payload.version);
}

export function usePwaRuntime(): PwaRuntimeState {
  const [installPrompt, setInstallPrompt] = useState<BeforeInstallPromptEvent | null>(null);
  const [installMode, setInstallMode] = useState<PwaInstallMode>(() => resolveInstallMode(null));
  const [isOffline, setIsOffline] = useState<boolean>(() =>
    typeof navigator === "undefined" ? false : !navigator.onLine,
  );
  const [shellReady, setShellReady] = useState<boolean>(() =>
    typeof navigator === "undefined" ? false : !!navigator.serviceWorker?.controller,
  );
  const [updateVisible, setUpdateVisible] = useState(false);
  const [availableVersion, setAvailableVersion] = useState<string | null>(null);
  const updateServiceWorkerRef = useRef<((reloadPage?: boolean) => Promise<void>) | null>(null);

  const currentVersion = useMemo(() => normalizeVersion(frontendVersion), []);

  useEffect(() => {
    if (typeof window === "undefined") return undefined;

    const mediaQueries = [
      window.matchMedia("(display-mode: standalone)"),
      window.matchMedia("(display-mode: window-controls-overlay)"),
    ];

    const refreshMode = () => {
      setInstallMode(resolveInstallMode(installPrompt));
    };

    const handleBeforeInstallPrompt = (event: Event) => {
      event.preventDefault();
      setInstallPrompt(event as BeforeInstallPromptEvent);
      setInstallMode("prompt");
    };

    const handleAppInstalled = () => {
      setInstallPrompt(null);
      setInstallMode("installed");
    };

    const handleOffline = () => setIsOffline(true);
    const handleOnline = () => setIsOffline(false);

    window.addEventListener("beforeinstallprompt", handleBeforeInstallPrompt);
    window.addEventListener("appinstalled", handleAppInstalled);
    window.addEventListener("offline", handleOffline);
    window.addEventListener("online", handleOnline);

    for (const query of mediaQueries) {
      query.addEventListener("change", refreshMode);
    }

    refreshMode();

    return () => {
      window.removeEventListener("beforeinstallprompt", handleBeforeInstallPrompt);
      window.removeEventListener("appinstalled", handleAppInstalled);
      window.removeEventListener("offline", handleOffline);
      window.removeEventListener("online", handleOnline);
      for (const query of mediaQueries) {
        query.removeEventListener("change", refreshMode);
      }
    };
  }, [installPrompt]);

  useEffect(() => {
    if (typeof window === "undefined" || !("serviceWorker" in navigator)) return undefined;

    const intervalRefs: Array<ReturnType<typeof setInterval>> = [];

    updateServiceWorkerRef.current = registerSW({
      immediate: true,
      onOfflineReady() {
        setShellReady(true);
      },
      async onNeedRefresh() {
        try {
          setAvailableVersion(await fetchFrontendVersion());
        } catch {
          setAvailableVersion(null);
        }
        setUpdateVisible(true);
      },
      onNeedReload() {
        window.location.reload();
      },
      onRegisteredSW(_swUrl, registration) {
        if (!registration) return;
        intervalRefs.push(
          setInterval(() => {
            void registration.update();
          }, UPDATE_CHECK_INTERVAL_MS),
        );
      },
    });

    return () => {
      updateServiceWorkerRef.current = null;
      for (const interval of intervalRefs) clearInterval(interval);
    };
  }, []);

  const promptInstall = useCallback(async () => {
    if (!installPrompt) return;
    let accepted = false;
    await installPrompt.prompt();
    try {
      const choice = await installPrompt.userChoice;
      if (choice.outcome === "accepted") {
        accepted = true;
        setInstallPrompt(null);
        setInstallMode("installed");
      }
    } finally {
      if (!accepted) {
        setInstallMode(resolveInstallMode(null));
      }
    }
  }, [installPrompt]);

  const applyUpdate = useCallback(async () => {
    setUpdateVisible(false);
    await updateServiceWorkerRef.current?.(true);
  }, []);

  const dismissUpdate = useCallback(() => {
    setUpdateVisible(false);
  }, []);

  return {
    installMode,
    installSupported: installMode !== "unsupported",
    isOffline,
    shellReady,
    update: {
      currentVersion,
      availableVersion,
      visible: updateVisible,
    },
    promptInstall,
    applyUpdate,
    dismissUpdate,
  };
}

export default usePwaRuntime;
