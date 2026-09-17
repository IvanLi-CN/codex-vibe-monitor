/** @vitest-environment jsdom */
import * as __module0 from "react";
import * as __module1 from "react-dom/server";
import * as __module2 from "vitest";
import * as __module3 from "../../i18n";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import rawSuite from "./InvocationTable.test.part-9.raw.txt?raw";
import * as __module4 from "./InvocationTable.test-support";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  renderToStaticMarkup: __module1.renderToStaticMarkup,
  expect: __module2.expect,
  it: __module2.it,
  I18nProvider: __module3.I18nProvider,
  apiMocks: __module4.apiMocks,
  createInvocationRecord: __module4.createInvocationRecord,
  createRequestBodyFixture: __module4.createRequestBodyFixture,
  createResponseBodyFixture: __module4.createResponseBodyFixture,
  createWorkflowDetailFixture: __module4.createWorkflowDetailFixture,
  InvocationDetailProbe: __module4.InvocationDetailProbe,
  LONG_PROXY_NAME: __module4.LONG_PROXY_NAME,
  renderInteractiveTable: __module4.renderInteractiveTable,
  renderTable: __module4.renderTable,
  waitForCondition: __module4.waitForCondition,
});
