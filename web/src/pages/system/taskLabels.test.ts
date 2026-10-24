import { afterEach, describe, expect, it, vi } from "vitest";
import type { ManagedTask } from "../../lib/api";
import {
  managedTaskCompletionLabel,
  managedTaskNextTriggerLabel,
  managedTaskRunStatusLabel,
  managedTaskTriggerLabel,
} from "./taskLabels";

const timeZoneMock = vi.hoisted(() => ({ value: "Asia/Shanghai" }));

vi.mock("../../lib/timeZone", () => ({
  getBrowserTimeZone: () => timeZoneMock.value,
}));

const task: ManagedTask = {
  taskKey: "retention_archive",
  title: "数据保留与归档",
  description: "按保留策略归档并清理历史数据",
  triggerMode: "cron",
  enabled: true,
  isManual: false,
  cronExpr: "0 * * * *",
  nextTriggerAt: "2026-01-01T00:30:00.000Z",
};

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

describe("managed task time labels", () => {
  afterEach(() => {
    timeZoneMock.value = "Asia/Shanghai";
  });

  it("formats next trigger in the browser timezone, including a cross-day conversion", () => {
    expect(managedTaskNextTriggerLabel(task)).toContain("2026/01/01");
    expect(managedTaskNextTriggerLabel(task)).toContain("08:30:00");

    timeZoneMock.value = "UTC";
    expect(managedTaskNextTriggerLabel(task)).toContain("2026/01/01");
    expect(managedTaskNextTriggerLabel(task)).toContain("00:30:00");

    timeZoneMock.value = "America/Los_Angeles";
    expect(managedTaskNextTriggerLabel(task)).toContain("2025/12/31");
    expect(managedTaskNextTriggerLabel(task)).toContain("16:30:00");
  });

  it("keeps cron semantics explicitly marked as UTC", () => {
    expect(managedTaskTriggerLabel(task)).toBe("UTC 定时");
  });
});
