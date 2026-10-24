import { describe, expect, it } from "vitest";
import { managedTaskCompletionLabel, managedTaskRunStatusLabel } from "./taskLabels";

describe("managedTaskCompletionLabel", () => {
  it.each([
    ["no-work", "无待办"],
    ["complete", "本轮完成"],
    ["completed", "本轮完成"],
    ["partial", "部分完成"],
    ["deferred", "已延期"],
    ["failed", "本轮失败"],
  ])("maps %s to the stable operator label", (completion, label) => {
    expect(managedTaskCompletionLabel(completion)).toBe(label);
  });

  it("keeps missing completion evidence explicit", () => {
    expect(managedTaskCompletionLabel(null)).toBe("未知");
  });
});

describe("managedTaskRunStatusLabel", () => {
  it("does not present a partial run as success", () => {
    expect(managedTaskRunStatusLabel("skipped", "partial")).toBe("部分完成");
  });
});
