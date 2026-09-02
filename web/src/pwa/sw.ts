/// <reference lib="webworker" />

import { clientsClaim } from "workbox-core";
import {
  cleanupOutdatedCaches,
  createHandlerBoundToURL,
  precacheAndRoute,
} from "workbox-precaching";
import { NavigationRoute, registerRoute } from "workbox-routing";
import { NetworkOnly } from "workbox-strategies";

declare let self: ServiceWorkerGlobalScope & {
  __WB_MANIFEST: Array<{ url: string; revision: string | null }>;
};

function isRuntimeMetadataPath(pathname: string): boolean {
  const filename = pathname.slice(pathname.lastIndexOf("/") + 1);
  return filename === "site.webmanifest" || filename === "version.json";
}

function isInstallIconPath(pathname: string): boolean {
  const filename = pathname.slice(pathname.lastIndexOf("/") + 1);
  return /^(?:favicon|icon-192|icon-512|maskable-192|maskable-512)-[0-9a-f]{12}\.(?:png|svg)$/.test(
    filename,
  );
}

const manifestEntries = self.__WB_MANIFEST.filter((entry) => {
  const pathname = (typeof entry === "string" ? entry : entry.url).split(/[?#]/, 1)[0];
  return !isRuntimeMetadataPath(pathname) && !isInstallIconPath(pathname);
});

cleanupOutdatedCaches();
precacheAndRoute(manifestEntries);

const navigationHandler = createHandlerBoundToURL(`${import.meta.env.BASE_URL}index.html`);

registerRoute(
  new NavigationRoute(navigationHandler, {
    denylist: [/^\/api\//, /^\/events\//, /^\/__test\//],
  }),
);

// Runtime metadata and install icons must stay network-backed so new releases can be discovered.
registerRoute(
  ({ url }) => isRuntimeMetadataPath(url.pathname) || isInstallIconPath(url.pathname),
  new NetworkOnly(),
);

self.addEventListener("message", (event) => {
  if (event.data?.type === "SKIP_WAITING") {
    void self.skipWaiting();
  }
});

clientsClaim();
