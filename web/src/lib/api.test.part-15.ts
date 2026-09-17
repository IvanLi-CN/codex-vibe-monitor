import * as __module0 from "vitest";
import { loadRawTestSuite } from "../test-utils/loadRawTestSuite";
import * as __module1 from "./api";
import rawSuite from "./api.test.part-15.raw.txt?raw";

loadRawTestSuite(rawSuite, {
  expect: __module0.expect,
  it: __module0.it,
  vi: __module0.vi,
  fetchForwardProxyBindingNodes: __module1.fetchForwardProxyBindingNodes,
  fetchSettings: __module1.fetchSettings,
  fetchUpstreamAccounts: __module1.fetchUpstreamAccounts,
  updateProxySettings: __module1.updateProxySettings,
});
