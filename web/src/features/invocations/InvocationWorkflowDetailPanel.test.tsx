/** @vitest-environment jsdom */
import * as __module0 from "react";
import * as __module1 from "react-dom/client";
import * as __module2 from "vitest";
import * as __module3 from "../../lib/dashboardWorkingConversations";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module5 from "./InvocationWorkflowDetailPanel.fixtures";
import rawSuite from "./InvocationWorkflowDetailPanel.test.raw.txt?raw";

const scope: Record<string, unknown> = {
  act: __module0.act,
  createRoot: __module1.createRoot,
  afterEach: __module2.afterEach,
  beforeAll: __module2.beforeAll,
  beforeEach: __module2.beforeEach,
  expect: __module2.expect,
  it: __module2.it,
  vi: __module2.vi,
  formatDashboardWorkingConversationSequenceId:
    __module3.formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey: __module3.hashDashboardWorkingConversationKey,
  InvocationWorkflowDetailPanel: undefined,
  failedWorkflowFinalResponseBodyText: __module5.failedWorkflowFinalResponseBodyText,
  failedWorkflowRequestBodySize: __module5.failedWorkflowRequestBodySize,
  failedWorkflowRequestBodyText: __module5.failedWorkflowRequestBodyText,
  failedWorkflowResponseBody: __module5.failedWorkflowResponseBody,
  failedWorkflowResponseBodySize: __module5.failedWorkflowResponseBodySize,
  failedWorkflowResponseBodyText: __module5.failedWorkflowResponseBodyText,
};

loadRawTestSuite(rawSuite, scope);
scope.InvocationWorkflowDetailPanel = (
  await import("./InvocationWorkflowDetailPanel")
).InvocationWorkflowDetailPanel;
