/** @vitest-environment jsdom */
import * as __module0 from "@testing-library/dom";
import * as __module1 from "@testing-library/user-event";
import * as __module2 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module3 from "./DashboardWorkingConversationsSection.support";
import rawSuite from "./DashboardWorkingConversationsSection.test.part-11.raw.txt?raw";
import * as __module4 from "./dashboardBulkRouteBindPreferences";

loadRawTestSuite(rawSuite, {
  waitFor: __module0.waitFor,
  userEvent: __module1.default,
  expect: __module2.expect,
  it: __module2.it,
  vi: __module2.vi,
  BULK_BINDING_ACCOUNTS: __module3.BULK_BINDING_ACCOUNTS,
  createBulkConversationFetchMock: __module3.createBulkConversationFetchMock,
  createConversation: __module3.createConversation,
  createPreview: __module3.createPreview,
  createResponse: __module3.createResponse,
  host: __module3.host,
  renderSection: __module3.renderSection,
  requireTestValue: __module3.requireTestValue,
  DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY:
    __module4.DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
});
