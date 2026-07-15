import { describe, expect, it } from "vitest";
import { buildAccountPoolGroupSummaries } from "./accountPoolGroups";

describe("buildAccountPoolGroupSummaries", () => {
  it("keeps k12 in grouped plan counts ahead of pro/team", () => {
    const groups = buildAccountPoolGroupSummaries({
      items: [
        {
          id: 1,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Student One",
          groupName: "school",
          isMother: false,
          status: "active",
          enabled: true,
          planType: "k12",
        },
        {
          id: 2,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Student Two",
          groupName: "school",
          isMother: false,
          status: "active",
          enabled: true,
          planType: "pro",
        },
        {
          id: 3,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Student Three",
          groupName: "school",
          isMother: false,
          status: "active",
          enabled: true,
          planType: "team",
        },
      ],
      groups: [
        {
          groupName: "school",
          accountCount: 3,
          note: null,
          boundProxyKeys: [],
          nodeShuntEnabled: false,
          singleAccountRotationEnabled: false,
          upstream429RetryEnabled: false,
          upstream429MaxRetries: 0,
          concurrencyLimit: 0,
          routingRule: {
            allowCutOut: true,
            allowCutIn: true,
          },
        },
      ],
      forwardProxyNodes: [],
      ungroupedLabel: "Ungrouped",
      groupedPlanLabel: (planType) => (planType ? planType.toUpperCase() : null),
    });

    expect(groups).toHaveLength(1);
    expect(groups[0]?.planCounts.map((entry) => entry.key)).toEqual(["k12", "pro", "team"]);
    expect(groups[0]?.planCounts.map((entry) => entry.label)).toEqual(["K12", "PRO", "TEAM"]);
  });
});
