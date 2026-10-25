import type { DemoScene } from "./runtime";

export type DemoRouteCase = {
  path: string;
  expectedPath: string;
  landmark: { kind: "testid"; value: string } | { kind: "heading"; value: string };
};

export const demoRoutes: readonly DemoRouteCase[] = [
  {
    path: "/#/",
    expectedPath: "/dashboard",
    landmark: { kind: "testid", value: "dashboard-main-content" },
  },
  {
    path: "/#/dashboard",
    expectedPath: "/dashboard",
    landmark: { kind: "testid", value: "dashboard-main-content" },
  },
  {
    path: "/#/dashboard/invocations/demo-invocation-9002",
    expectedPath: "/dashboard/invocations/demo-invocation-9002",
    landmark: { kind: "testid", value: "dashboard-invocation-detail-drawer" },
  },
  { path: "/#/stats", expectedPath: "/stats", landmark: { kind: "heading", value: "统计" } },
  { path: "/#/live", expectedPath: "/live", landmark: { kind: "testid", value: "live-view-tabs" } },
  {
    path: "/#/records",
    expectedPath: "/records",
    landmark: { kind: "testid", value: "records-filters-panel" },
  },
  {
    path: "/#/account-pool",
    expectedPath: "/account-pool/transits",
    landmark: { kind: "heading", value: "中转" },
  },
  {
    path: "/#/account-pool/upstream-accounts",
    expectedPath: "/account-pool/pool",
    landmark: { kind: "heading", value: "号池" },
  },
  {
    path: "/#/account-pool/transits",
    expectedPath: "/account-pool/transits",
    landmark: { kind: "heading", value: "中转" },
  },
  {
    path: "/#/account-pool/pool",
    expectedPath: "/account-pool/pool",
    landmark: { kind: "heading", value: "号池" },
  },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=apiKey",
    expectedPath: "/account-pool/transits/new",
    landmark: { kind: "heading", value: "新增账号" },
  },
  {
    path: "/#/account-pool/upstream-accounts/new?mode=oauth",
    expectedPath: "/account-pool/pool/new",
    landmark: { kind: "heading", value: "新增账号" },
  },
  {
    path: "/#/account-pool/maintenance-records",
    expectedPath: "/account-pool/maintenance-records",
    landmark: { kind: "heading", value: "非模型调用执行记录" },
  },
  {
    path: "/#/account-pool/groups",
    expectedPath: "/account-pool/groups",
    landmark: { kind: "testid", value: "account-pool-groups-list" },
  },
  {
    path: "/#/system",
    expectedPath: "/system/status",
    landmark: { kind: "testid", value: "system-status-overview" },
  },
  {
    path: "/#/system/status",
    expectedPath: "/system/status",
    landmark: { kind: "testid", value: "system-status-overview" },
  },
  {
    path: "/#/system/performance",
    expectedPath: "/system/performance",
    landmark: { kind: "testid", value: "system-observability-page" },
  },
  {
    path: "/#/system/tasks",
    expectedPath: "/system/tasks",
    landmark: { kind: "testid", value: "system-tasks-list" },
  },
  {
    path: "/#/system/tasks/retention_archive",
    expectedPath: "/system/tasks/retention_archive",
    landmark: { kind: "heading", value: "数据保留与归档" },
  },
  {
    path: "/#/system/prompt-cache",
    expectedPath: "/system/prompt-cache",
    landmark: { kind: "testid", value: "system-prompt-cache" },
  },
  {
    path: "/#/system/settings",
    expectedPath: "/system/settings",
    landmark: { kind: "heading", value: "系统设置" },
  },
  {
    path: "/#/system/models",
    expectedPath: "/system/models",
    landmark: { kind: "heading", value: "模型" },
  },
  {
    path: "/#/system/proxy",
    expectedPath: "/system/proxy",
    landmark: { kind: "heading", value: "代理" },
  },
  {
    path: "/#/settings",
    expectedPath: "/system/settings",
    landmark: { kind: "heading", value: "系统设置" },
  },
  {
    path: "/#/settings/legacy",
    expectedPath: "/system/settings",
    landmark: { kind: "heading", value: "系统设置" },
  },
  {
    path: "/#/not-a-route",
    expectedPath: "/dashboard",
    landmark: { kind: "testid", value: "dashboard-main-content" },
  },
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
