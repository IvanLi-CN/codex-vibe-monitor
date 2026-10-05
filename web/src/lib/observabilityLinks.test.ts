import { describe, expect, it } from "vitest";
import { grafanaLink } from "./observabilityLinks";

describe("Grafana link contract", () => {
  it("encodes task keys without adding credentials or arbitrary parameters", () => {
    const result = grafanaLink(
      "https://grafana.example.invalid/subpath",
      "cvm-runtime",
      "retention / archive",
    );
    const url = new URL(result ?? "");
    expect(url.pathname).toBe("/subpath/d/cvm-runtime");
    expect(url.searchParams.get("var-task_key")).toBe("retention / archive");
    expect(url.searchParams.get("timezone")).toBe("utc");
  });
  it("rejects untrusted origins and dashboard identities", () => {
    for (const base of [
      "http://grafana.example.invalid",
      "https://secret@grafana.example.invalid",
      "https://grafana.example.invalid/?token=secret",
    ])
      expect(grafanaLink(base, "cvm-overview")).toBeUndefined();
    expect(grafanaLink("https://grafana.example.invalid", "unexpected")).toBeUndefined();
  });
});
