/** @vitest-environment jsdom */
import * as __module0 from "react";
import * as __module1 from "vitest";
import * as __module2 from "../lib/invocationRecords";
import { loadRawTestSuite } from "../test-utils/loadRawTestSuite";
import rawSuite from "./useInvocationRecords.test.raw.txt?raw";
import * as __module3 from "./useInvocationRecords.test-support";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  expect: __module1.expect,
  it: __module1.it,
  vi: __module1.vi,
  RECORDS_NEW_COUNT_POLL_INTERVAL_MS: __module2.RECORDS_NEW_COUNT_POLL_INTERVAL_MS,
  apiMocks: __module3.apiMocks,
  click: __module3.click,
  createListResponse: __module3.createListResponse,
  createNewCountResponse: __module3.createNewCountResponse,
  createRecord: __module3.createRecord,
  createSummaryResponse: __module3.createSummaryResponse,
  emitRealtimeRecords: __module3.emitRealtimeRecords,
  flushAsync: __module3.flushAsync,
  Probe: __module3.Probe,
  render: __module3.render,
  text: __module3.text,
  waitFor: __module3.waitFor,
});
