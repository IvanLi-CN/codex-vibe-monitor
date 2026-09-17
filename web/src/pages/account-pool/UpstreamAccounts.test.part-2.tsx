/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck
import * as __module0 from "react";
import * as __module1 from "react-dom/client";
import * as __module2 from "react-router-dom";
import * as __module3 from "vitest";
import * as __module4 from "../../components/ui/system-notifications";
import * as __module5 from "../../i18n";
import { loadRawTestSuite } from "../../test-utils/loadRawTestSuite";
import * as __module6 from "../../theme/context";
import * as __module7 from "./UpstreamAccounts";
import rawSuite from "./UpstreamAccounts.test.part-2.raw.txt?raw";
import * as __module8 from "./UpstreamAccounts.test-support";

loadRawTestSuite(rawSuite, {
  act: __module0.act,
  createRoot: __module1.createRoot,
  MemoryRouter: __module2.MemoryRouter,
  expect: __module3.expect,
  it: __module3.it,
  vi: __module3.vi,
  SystemNotificationProvider: __module4.SystemNotificationProvider,
  I18nProvider: __module5.I18nProvider,
  ThemeProvider: __module6.ThemeProvider,
  SharedUpstreamAccountDetailDrawer: __module7.SharedUpstreamAccountDetailDrawer,
  clickButton: __module8.clickButton,
  clickTab: __module8.clickTab,
  defaultEffectiveRoutingRule: __module8.defaultEffectiveRoutingRule,
  expectRosterHookQuery: __module8.expectRosterHookQuery,
  findButton: __module8.findButton,
  flushAsync: __module8.flushAsync,
  hookMocks: __module8.hookMocks,
  host: __module8.host,
  mockAccountsPage: __module8.mockAccountsPage,
  mockRosterFreshnessPage: __module8.mockRosterFreshnessPage,
  navigateMock: __module8.navigateMock,
  readStoredUpstreamFilters: __module8.readStoredUpstreamFilters,
  render: __module8.render,
  root: __module8.root,
  setCompactViewportMatch: __module8.setCompactViewportMatch,
  setHost: __module8.setHost,
  setRoot: __module8.setRoot,
  writeStoredUpstreamFilters: __module8.writeStoredUpstreamFilters,
});
