export interface AppNavigationItem {
  to: string;
  labelKey: string;
  matchPrefixes?: string[];
}

export interface AppNavigationGroup {
  to: string;
  labelKey: string;
  items: AppNavigationItem[];
  matchPrefixes?: string[];
}

export interface ResolvedAppNavigation {
  topLevelItem: AppNavigationItem | AppNavigationGroup;
  nestedItem: AppNavigationItem | null;
  nestedGroup: AppNavigationGroup | null;
}

export const topLevelNavItems: AppNavigationGroup[] = [
  {
    to: "/dashboard",
    labelKey: "app.nav.dashboard",
    items: [],
  },
  {
    to: "/stats",
    labelKey: "app.nav.stats",
    items: [],
  },
  {
    to: "/live",
    labelKey: "app.nav.live",
    items: [],
  },
  {
    to: "/records",
    labelKey: "app.nav.records",
    items: [],
  },
  {
    to: "/account-pool",
    labelKey: "app.nav.accountPool",
    matchPrefixes: ["/account-pool"],
    items: [
      {
        to: "/account-pool/upstream-accounts",
        labelKey: "accountPool.nav.upstreamAccounts",
        matchPrefixes: ["/account-pool/upstream-accounts"],
      },
      {
        to: "/account-pool/groups",
        labelKey: "accountPool.nav.groups",
        matchPrefixes: ["/account-pool/groups"],
      },
      {
        to: "/account-pool/maintenance-records",
        labelKey: "accountPool.nav.maintenanceRecords",
        matchPrefixes: ["/account-pool/maintenance-records"],
      },
    ],
  },
  {
    to: "/system",
    labelKey: "app.nav.system",
    matchPrefixes: ["/system", "/settings", "/settings/legacy"],
    items: [
      {
        to: "/system/status",
        labelKey: "system.nav.status",
        matchPrefixes: ["/system/status"],
      },
      {
        to: "/system/tasks",
        labelKey: "system.nav.tasks",
        matchPrefixes: ["/system/tasks"],
      },
      {
        to: "/system/settings",
        labelKey: "system.nav.settings",
        matchPrefixes: ["/system/settings", "/settings", "/settings/legacy"],
      },
      {
        to: "/system/proxy",
        labelKey: "system.nav.proxy",
        matchPrefixes: ["/system/proxy"],
      },
    ],
  },
];

export const desktopNavItems = topLevelNavItems.map(({ to, labelKey, matchPrefixes }) => ({
  to,
  labelKey,
  matchPrefixes,
}));

export const accountPoolNavItems = topLevelNavItems[4].items;
export const systemNavItems = topLevelNavItems[5].items;
export const mobileNavigationGroups = topLevelNavItems;

function normalizePrefix(prefix: string) {
  return prefix.endsWith("/") ? prefix.slice(0, -1) : prefix;
}

export function matchesNavigationPath(
  pathname: string,
  item: AppNavigationItem | AppNavigationGroup,
) {
  const prefixes = item.matchPrefixes ?? [item.to];
  return prefixes.some((prefix) => {
    const normalizedPrefix = normalizePrefix(prefix);
    return pathname === normalizedPrefix || pathname.startsWith(`${normalizedPrefix}/`);
  });
}

export function resolveAppNavigation(pathname: string): ResolvedAppNavigation {
  const matchedTopLevelItem =
    topLevelNavItems.find((item) => matchesNavigationPath(pathname, item)) ?? topLevelNavItems[0];

  const matchedNestedItem =
    matchedTopLevelItem.items.find((item) => matchesNavigationPath(pathname, item)) ?? null;

  return {
    topLevelItem: matchedTopLevelItem,
    nestedItem: matchedNestedItem,
    nestedGroup: matchedNestedItem ? matchedTopLevelItem : null,
  };
}
