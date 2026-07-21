/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";
import * as structuredPayloadModule from "./structuredPayload";

const labels = {
  json: "JSON",
  ndjson: "NDJSON",
  sse: "SSE event stream",
  text: "Plain text",
  largePayload: "This payload is larger than 1 MiB. Raw text is shown to protect the interface.",
  parseLargePayload: "Parse structured content",
  event: "Event",
  data: "Data",
  expand: "Expand JSON",
  collapse: "Collapse JSON",
};

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

function render(value: string) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(<StructuredPayloadViewer value={value} labels={labels} />);
  });
}

function rerender(value: string) {
  act(() => {
    root?.render(<StructuredPayloadViewer value={value} labels={labels} />);
  });
}

function parseButton() {
  return host?.querySelector("button");
}

function parsedViewer() {
  return host?.querySelector('[data-testid="structured-payload-viewer"]');
}

describe("StructuredPayloadViewer", () => {
  it("wraps parsed payloads in an overflow boundary", () => {
    render(
      JSON.stringify({
        status: "completed",
        trace: `trace-${"0123456789abcdef".repeat(32)}`,
      }),
    );

    expect(parsedViewer()?.className).toContain("overflow-hidden");
    expect(host?.querySelector(".structured-payload-scroll")).not.toBeNull();
  });

  it("resets large payload parse consent when the payload value changes", async () => {
    const parseSpy = vi.spyOn(structuredPayloadModule, "parseStructuredPayload");
    const firstPayload = JSON.stringify({ payload: "x".repeat(1024 * 1024) });
    const secondPayload = JSON.stringify({ payload: "y".repeat(1024 * 1024) });

    render(firstPayload);
    expect(parseButton()?.textContent).toContain(labels.parseLargePayload);
    expect(parsedViewer()).toBeNull();

    await act(async () => {
      parseButton()?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(parsedViewer()?.getAttribute("data-payload-kind")).toBe("json");
    expect(parseButton()).toBeNull();
    parseSpy.mockClear();

    rerender(secondPayload);

    expect(parseSpy).not.toHaveBeenCalled();
    expect(parseButton()?.textContent).toContain(labels.parseLargePayload);
    expect(parsedViewer()).toBeNull();
  });
});
