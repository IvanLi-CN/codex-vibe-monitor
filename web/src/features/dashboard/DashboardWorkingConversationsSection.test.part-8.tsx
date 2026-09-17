import * as __module0 from "react";
import * as __module1 from "vitest";
import * as __module2 from "../../lib/dashboardWorkingConversations";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module3 from "./DashboardWorkingConversationsSection.support";
import rawSuite from "./DashboardWorkingConversationsSection.test.part-8.raw.txt?raw";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  expect: __module1.expect,
  it: __module1.it,
  vi: __module1.vi,
  mapPromptCacheConversationsToDashboardCards:
    __module2.mapPromptCacheConversationsToDashboardCards,
  createConversation: __module3.createConversation,
  createPreview: __module3.createPreview,
  createResponse: __module3.createResponse,
  host: __module3.host,
  renderSection: __module3.renderSection,
  renderSectionWithCards: __module3.renderSectionWithCards,
  requireTestValue: __module3.requireTestValue,
  rerenderSection: __module3.rerenderSection,
  rerenderSectionWithCards: __module3.rerenderSectionWithCards,
  virtualizerMocks: __module3.virtualizerMocks,
});
