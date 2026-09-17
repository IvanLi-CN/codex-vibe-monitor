/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck

import { expect, it } from "vitest";
import { flushAsync, mockAccountsPage, render } from "./UpstreamAccounts.test-support";

it("opens the shared drawer on the routing tab when the query requests routing", async () => {
  mockAccountsPage();
  render("/account-pool/upstream-accounts?upstreamAccountId=5&upstreamAccountTab=routing");
  await flushAsync();

  expect(document.body.textContent ?? "").toMatch(/最终生效规则|Effective routing rule/);
  expect(document.body.textContent ?? "").toMatch(/字段来源明细|Field source breakdown/);
});
it("defaults to the overview tab when only the account id is present", async () => {
  mockAccountsPage();
  render("/account-pool/upstream-accounts?upstreamAccountId=5");
  await flushAsync();

  const overviewTab = Array.from(document.body.querySelectorAll('button[role="tab"]')).find(
    (candidate) => /概览|overview/i.test(candidate.textContent ?? ""),
  );
  if (!(overviewTab instanceof HTMLButtonElement)) {
    throw new Error("missing overview tab");
  }

  expect(overviewTab.getAttribute("aria-selected")).toBe("true");
});
/** @vitest-environment jsdom */
