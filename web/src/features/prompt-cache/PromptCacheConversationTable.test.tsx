/** @vitest-environment jsdom */
import * as __module0 from "react";
import * as __module1 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module2 from "./PromptCacheConversationTable";
import rawSuite from "./PromptCacheConversationTable.test.raw.txt?raw";
import * as __module3 from "./PromptCacheConversationTable.test-support";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  useState: __module0.useState,
  expect: __module1.expect,
  it: __module1.it,
  vi: __module1.vi,
  PromptCacheConversationTable: __module2.PromptCacheConversationTable,
  createConversation: __module3.createConversation,
  findButtonByAriaLabel: __module3.findButtonByAriaLabel,
  formatZhDateTime: __module3.formatZhDateTime,
  get host() {
    return __module3.host;
  },
  renderInteractive: __module3.renderInteractive,
  renderInteractiveElement: __module3.renderInteractiveElement,
  renderTable: __module3.renderTable,
});
