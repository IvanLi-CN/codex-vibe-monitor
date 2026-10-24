import type { DemoScene } from "./runtime";

export type DemoRouteCase = {
  path: string;
  expectedPath: string;
};

export const demoRoutes: readonly DemoRouteCase[] = [
  { path: "/#/", expectedPath: "/dashboard" },
  { path: "/#/dashboard", expectedPath: "/dashboard" },
  {
    path: "/#/dashboard/invocations/demo-invocation-9002",
    expectedPath: "/dashboard/invocations/demo-invocation-9002",
  },
  { path: "/#/stats", expectedPath: "/stats" },
  { path: "/#/live", expectedPath: "/live" },
  { path: "/#/records", expectedPath: "/records" },
  { path: "/#/account-pool", expectedPath: "/account-pool/transits" },
  { path: "/#/account-pool/upstream-accounts", expectedPath: "/account-pool/pool" },
  { path: "/#/account-pool/transits", expectedPath: "/account-pool/transits" },
  { path: "/#/account-pool/pool", expectedPath: "/account-pool/pool" },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=apiKey",
    expectedPath: "/account-pool/transits/new",
  },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=oauth",
    expectedPath: "/account-pool/pool/new",
  },
  {
    path: "/#/account-pool/maintenance-records",
    expectedPath: "/account-pool/maintenance-records",
  },
  { path: "/#/account-pool/groups", expectedPath: "/account-pool/groups" },
  { path: "/#/system", expectedPath: "/system/status" },
  { path: "/#/system/status", expectedPath: "/system/status" },
  { path: "/#/system/performance", expectedPath: "/system/performance" },
  { path: "/#/system/tasks", expectedPath: "/system/tasks" },
  {
    path: "/#/system/tasks/retention_archive",
    expectedPath: "/system/tasks/retention_archive",
  },
  { path: "/#/system/prompt-cache", expectedPath: "/system/prompt-cache" },
  { path: "/#/system/settings", expectedPath: "/system/settings" },
  { path: "/#/system/models", expectedPath: "/system/models" },
  { path: "/#/system/proxy", expectedPath: "/system/proxy" },
  { path: "/#/settings", expectedPath: "/system/settings" },
  { path: "/#/settings/legacy", expectedPath: "/system/settings" },
  { path: "/#/not-a-route", expectedPath: "/dashboard" },
] as const;

export const demoScenes: readonly DemoScene[] = [
  "operational",
  "attention",
  "empty",
  "network-failure",
];

export const demoBrowserRepresentativeRoutes = [
  "/#/dashboard",
  "/#/records",
  "/#/account-pool/transits",
  "/#/system/status",
] as const;

export const demoSemanticMatrix = demoRoutes.flatMap((route) =>
  demoScenes.map((scene) => ({ route, scene })),
);
