type RegisterSwOptions = {
  onNeedRefresh?: () => void | Promise<void>;
  onNeedReload?: () => void;
  onOfflineReady?: () => void;
  onRegisteredSW?: (swUrl: string, registration: ServiceWorkerRegistration | undefined) => void;
};

export function registerSW(options: RegisterSwOptions = {}) {
  queueMicrotask(() => {
    options.onRegisteredSW?.("/sw.js", undefined);
  });

  return async (_reloadPage?: boolean) => undefined;
}

export default registerSW;
