/** @vitest-environment jsdom */
import * as __module0 from "@testing-library/user-event";
import * as __module1 from "react";
import * as __module2 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import rawSuite from "./PromptCacheConversationTable.test.part-3.raw.txt?raw";
import * as __module3 from "./PromptCacheConversationTable.test-support";

loadRawTestSuite(rawSuite, {
  userEvent: __module0.default,
  act: __module1.act,
  expect: __module2.expect,
  it: __module2.it,
  vi: __module2.vi,
  apiMocks: __module3.apiMocks,
  clickDrawerTab: __module3.clickDrawerTab,
  createConversation: __module3.createConversation,
  createUpstreamAccountSummary: __module3.createUpstreamAccountSummary,
  detailTopicMocks: __module3.detailTopicMocks,
  findButtonByAriaLabel: __module3.findButtonByAriaLabel,
  findSelectOption: __module3.findSelectOption,
  flushInteractive: __module3.flushInteractive,
  renderInteractive: __module3.renderInteractive,
  requireTestValue: __module3.requireTestValue,
});
