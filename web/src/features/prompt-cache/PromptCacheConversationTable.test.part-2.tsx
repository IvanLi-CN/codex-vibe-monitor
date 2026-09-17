import * as __module0 from "react";
import * as __module1 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module2 from "./PromptCacheConversationTable";
import rawSuite from "./PromptCacheConversationTable.test.part-2.raw.txt?raw";
import * as __module3 from "./PromptCacheConversationTable.test-support";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  expect: __module1.expect,
  it: __module1.it,
  vi: __module1.vi,
  PromptCacheConversationHistoryDrawer: __module2.PromptCacheConversationHistoryDrawer,
  apiMocks: __module3.apiMocks,
  clickDrawerTab: __module3.clickDrawerTab,
  createConversation: __module3.createConversation,
  detailTopicMocks: __module3.detailTopicMocks,
  findButtonByAriaLabel: __module3.findButtonByAriaLabel,
  flushInteractive: __module3.flushInteractive,
  renderInteractive: __module3.renderInteractive,
  renderInteractiveElement: __module3.renderInteractiveElement,
});
