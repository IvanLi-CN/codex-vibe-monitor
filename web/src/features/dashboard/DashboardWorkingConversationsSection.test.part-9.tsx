/** @vitest-environment jsdom */
import * as __module0 from "@testing-library/dom";
import * as __module1 from "react";
import * as __module2 from "vitest";
import * as __module3 from "../../lib/dashboardWorkingConversations";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module4 from "./DashboardWorkingConversationsSection.support";
import rawSuite from "./DashboardWorkingConversationsSection.test.part-9.raw.txt?raw";

loadRawTestSuite(rawSuite, {
  fireEvent: __module0.fireEvent,
  waitFor: __module0.waitFor,
  act: __module1.act,
  expect: __module2.expect,
  it: __module2.it,
  vi: __module2.vi,
  mapPromptCacheConversationsToDashboardCards:
    __module3.mapPromptCacheConversationsToDashboardCards,
  createConversation: __module4.createConversation,
  createPreview: __module4.createPreview,
  createResponse: __module4.createResponse,
  createUpstreamAccountActivityResponse: __module4.createUpstreamAccountActivityResponse,
  host: __module4.host,
  LONG_ERROR_SUMMARY: __module4.LONG_ERROR_SUMMARY,
  renderSection: __module4.renderSection,
  renderSectionWithCards: __module4.renderSectionWithCards,
  requireTestValue: __module4.requireTestValue,
  rerenderSectionWithCards: __module4.rerenderSectionWithCards,
  upstreamAccountActivityMock: __module4.upstreamAccountActivityMock,
  virtualizerMocks: __module4.virtualizerMocks,
});
