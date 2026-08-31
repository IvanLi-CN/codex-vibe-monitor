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

const manifestEntries = self.__WB_MANIFEST.filter((entry) => {
  const pathname = (typeof entry === "string" ? entry : entry.url).split(/[?#]/, 1)[0];
  return !isRuntimeMetadataPath(pathname);
});

cleanupOutdatedCaches();
precacheAndRoute(manifestEntries);

const navigationHandler = createHandlerBoundToURL(`${import.meta.env.BASE_URL}index.html`);

registerRoute(
  new NavigationRoute(navigationHandler, {
    denylist: [/^\/api\//, /^\/events\//, /^\/__test\//],
  }),
);

// Metadata must be revalidated so a new manifest can point installed clients at new icon URLs.
registerRoute(({ url }) => isRuntimeMetadataPath(url.pathname), new NetworkOnly());

self.addEventListener("message", (event) => {
  if (event.data?.type === "SKIP_WAITING") {
    void self.skipWaiting();
  }
});

clientsClaim();
