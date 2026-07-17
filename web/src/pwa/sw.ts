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

const manifestEntries = self.__WB_MANIFEST.filter((entry) =>
  typeof entry === "string" ? !entry.endsWith("version.json") : !entry.url.endsWith("version.json"),
);

cleanupOutdatedCaches();
precacheAndRoute(manifestEntries);

const navigationHandler = createHandlerBoundToURL(`${import.meta.env.BASE_URL}index.html`);

registerRoute(
  new NavigationRoute(navigationHandler, {
    denylist: [/^\/api\//, /^\/events\//, /^\/__test\//],
  }),
);

registerRoute(({ url }) => url.pathname.endsWith("/version.json"), new NetworkOnly());

self.addEventListener("message", (event) => {
  if (event.data?.type === "SKIP_WAITING") {
    void self.skipWaiting();
  }
});

clientsClaim();
