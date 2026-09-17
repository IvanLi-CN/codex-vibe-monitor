/** @vitest-environment jsdom */
import * as __module0 from "react";
import * as __module1 from "react-dom/client";
import * as __module2 from "react-dom/server";
import * as __module3 from "vitest";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module4 from "./UpstreamAccountsTable";
import rawSuite from "./UpstreamAccountsTable.test.raw.txt?raw";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  createRoot: __module1.createRoot,
  renderToStaticMarkup: __module2.renderToStaticMarkup,
  afterEach: __module3.afterEach,
  describe: __module3.describe,
  expect: __module3.expect,
  it: __module3.it,
  vi: __module3.vi,
  resolveRosterActionableStatusChips: __module4.resolveRosterActionableStatusChips,
  resolveRosterSummaryStatusChips: __module4.resolveRosterSummaryStatusChips,
  resolveRoutingBlockCountdown: __module4.resolveRoutingBlockCountdown,
  UpstreamAccountsTable: __module4.UpstreamAccountsTable,
});
