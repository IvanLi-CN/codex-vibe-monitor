/** @vitest-environment jsdom */
import * as __module0 from "@testing-library/dom";
import * as __module1 from "@testing-library/user-event";
import * as __module2 from "react";
import * as __module3 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module4 from "./DashboardWorkingConversationsSection.support";
import rawSuite from "./DashboardWorkingConversationsSection.test.part-10.raw.txt?raw";
import * as __module5 from "./dashboardBulkRouteBindPreferences";

loadRawTestSuite(rawSuite, {
  fireEvent: __module0.fireEvent,
  waitFor: __module0.waitFor,
  userEvent: __module1.default,
  act: __module2.act,
  expect: __module3.expect,
  it: __module3.it,
  vi: __module3.vi,
  createBulkConversationFetchMock: __module4.createBulkConversationFetchMock,
  createConversation: __module4.createConversation,
  createPreview: __module4.createPreview,
  createResponse: __module4.createResponse,
  createUpstreamAccountActivityResponse: __module4.createUpstreamAccountActivityResponse,
  host: __module4.host,
  LONG_ERROR_SUMMARY: __module4.LONG_ERROR_SUMMARY,
  renderSection: __module4.renderSection,
  requireTestValue: __module4.requireTestValue,
  upstreamAccountActivityMock: __module4.upstreamAccountActivityMock,
  DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY:
    __module5.DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
});
