import { describe, expect, it } from "vitest";
import { translations } from "./translations";

describe("translations", () => {
  it("localizes chart non-success labels per locale", () => {
    expect(translations.en["chart.nonSuccess"]).toBe("Non-success");
    expect(translations.zh["chart.nonSuccess"]).toBe("非成功");
  });
});
