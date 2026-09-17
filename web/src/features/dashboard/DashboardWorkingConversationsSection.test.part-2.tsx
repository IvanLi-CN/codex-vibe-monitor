/** @vitest-environment jsdom */
import * as __module0 from "@testing-library/dom";
import * as __module1 from "react";
import * as __module2 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module3 from "./DashboardWorkingConversationsSection.support";
import rawSuite from "./DashboardWorkingConversationsSection.test.part-2.raw.txt?raw";

loadRawTestSuite(rawSuite, {
  fireEvent: __module0.fireEvent,
  waitFor: __module0.waitFor,
  act: __module1.act,
  expect: __module2.expect,
  it: __module2.it,
  vi: __module2.vi,
  createConversation: __module3.createConversation,
  createPreview: __module3.createPreview,
  createResponse: __module3.createResponse,
  createUpstreamAccountActivityResponse: __module3.createUpstreamAccountActivityResponse,
  host: __module3.host,
  renderSection: __module3.renderSection,
  rerenderSection: __module3.rerenderSection,
  upstreamAccountActivityMock: __module3.upstreamAccountActivityMock,
});
