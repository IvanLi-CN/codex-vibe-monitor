/** @vitest-environment jsdom */
import * as __module0 from "react";
import * as __module1 from "react-dom/client";
import * as __module2 from "react-router-dom";
import * as __module3 from "vitest";
import * as __module4 from "../lib/dashboardPerformanceDiagnostics";
import { loadRawTestSuite } from "../test-utils/loadRawTestSuite";
import rawSuite from "./Dashboard.test.raw.txt?raw";

const scope: Record<string, unknown> = {
  act: __module0.act,
  createRoot: __module1.createRoot,
  MemoryRouter: __module2.MemoryRouter,
  useLocation: __module2.useLocation,
  afterEach: __module3.afterEach,
  beforeAll: __module3.beforeAll,
  expect: __module3.expect,
  it: __module3.it,
  vi: __module3.vi,
  DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY:
    __module4.DASHBOARD_PERFORMANCE_DIAGNOSTICS_STORAGE_KEY,
  getDashboardPerformanceDiagnosticsSnapshot: __module4.getDashboardPerformanceDiagnosticsSnapshot,
  publishWorkingConversationPatchMetrics: __module4.publishWorkingConversationPatchMetrics,
  recordTodayChartRender: __module4.recordTodayChartRender,
  recordTodaySummaryRefresh: __module4.recordTodaySummaryRefresh,
  resetDashboardPerformanceDiagnostics: __module4.resetDashboardPerformanceDiagnostics,
  DashboardPage: undefined,
};

loadRawTestSuite(rawSuite, scope);
scope.DashboardPage = (await import("./Dashboard")).default;
