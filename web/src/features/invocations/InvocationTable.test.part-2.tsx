import { expect, it } from "vitest";
import { isInvocationPoolAccountRoutingInProgress } from "../../lib/invocation";
import { buildDetailViewForAccount, createInvocationRecord } from "./InvocationTable.test-support";

it("marks only running assigned pool accounts as routing in progress", () => {
  const runningAssigned = createInvocationRecord(0);
  runningAssigned.routeMode = "pool";
  runningAssigned.status = "running";
  runningAssigned.upstreamAccountId = 42;
  runningAssigned.upstreamAccountName = "Pool Alpha";

  const pendingUnassigned = createInvocationRecord(1);
  pendingUnassigned.routeMode = "pool";
  pendingUnassigned.status = "pending";
  pendingUnassigned.upstreamAccountId = null;
  pendingUnassigned.upstreamAccountName = undefined;

  const terminalAssigned = createInvocationRecord(2);
  terminalAssigned.routeMode = "pool";
  terminalAssigned.status = "success";
  terminalAssigned.upstreamAccountId = 42;
  terminalAssigned.upstreamAccountName = "Pool Alpha";

  expect(buildDetailViewForAccount(runningAssigned)).toMatchObject({
    accountLabel: "Pool Alpha",
    accountRoutingInProgress: true,
  });
  expect(buildDetailViewForAccount(pendingUnassigned)).toMatchObject({
    accountLabel: "table.account.poolRoutingPending",
    accountRoutingInProgress: false,
  });
  expect(buildDetailViewForAccount(terminalAssigned)).toMatchObject({
    accountLabel: "Pool Alpha",
    accountRoutingInProgress: false,
  });
  expect(isInvocationPoolAccountRoutingInProgress("pool", "running", null, 42)).toBe(true);
});
