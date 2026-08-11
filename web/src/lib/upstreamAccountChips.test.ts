import { describe, expect, it } from "vitest";
import {
  compactUpstreamPlanLabel,
  shouldShowUpstreamPlanChip,
  upstreamPlanChipRecipe,
} from "./upstreamAccountChips";

describe("upstreamAccountChips", () => {
  it("renders k12 as a first-class known plan badge", () => {
    expect(shouldShowUpstreamPlanChip("k12")).toBe(true);
    expect(compactUpstreamPlanLabel("k12")).toBe("K12");
    expect(upstreamPlanChipRecipe("k12")).toMatchObject({
      tone: "success",
      dataPlan: "k12",
    });
  });
});
