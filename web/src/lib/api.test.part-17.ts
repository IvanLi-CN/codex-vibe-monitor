import * as __module0 from "vitest";
import { loadRawTestSuite } from "../test-utils/loadRawTestSuite";
import * as __module1 from "./api";
import rawSuite from "./api.test.part-17.raw.txt?raw";

loadRawTestSuite(rawSuite, {
  expect: __module0.expect,
  it: __module0.it,
  vi: __module0.vi,
  bulkUpdatePromptCacheConversationBindings: __module1.bulkUpdatePromptCacheConversationBindings,
  fetchPromptCacheConversationBinding: __module1.fetchPromptCacheConversationBinding,
  fetchPromptCacheConversations: __module1.fetchPromptCacheConversations,
  fetchUpstreamAccounts: __module1.fetchUpstreamAccounts,
  fetchUpstreamStickyConversations: __module1.fetchUpstreamStickyConversations,
  updatePoolRoutingSettings: __module1.updatePoolRoutingSettings,
  updatePromptCacheConversationBinding: __module1.updatePromptCacheConversationBinding,
});
