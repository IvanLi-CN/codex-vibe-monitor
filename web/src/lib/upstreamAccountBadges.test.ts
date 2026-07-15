import { describe, expect, it } from "vitest";
import {
  compactUpstreamPlanLabel,
  shouldShowUpstreamPlanBadge,
  upstreamPlanBadgeRecipe,
} from "./upstreamAccountBadges";

describe("upstreamAccountBadges", () => {
  it("renders k12 as a first-class known plan badge", () => {
    expect(shouldShowUpstreamPlanBadge("k12")).toBe(true);
    expect(compactUpstreamPlanLabel("k12")).toBe("K12");
    expect(upstreamPlanBadgeRecipe("k12")).toMatchObject({
      variant: "success",
      dataPlan: "k12",
      className: "upstream-plan-badge",
    });
  });
});
