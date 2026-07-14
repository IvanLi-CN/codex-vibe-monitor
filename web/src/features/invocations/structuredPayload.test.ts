import { describe, expect, it } from "vitest";
import { getUtf8ByteLength, parseStructuredPayload } from "./structuredPayload";

describe("parseStructuredPayload", () => {
  it("parses JSON objects", () => {
    expect(parseStructuredPayload('{"ok":true,"items":[1,2]}')).toEqual({
      kind: "json",
      value: { ok: true, items: [1, 2] },
    });
  });

  it("parses strict NDJSON without accepting mixed logs", () => {
    expect(parseStructuredPayload('{"id":1}\n{"id":2}')).toEqual({
      kind: "ndjson",
      values: [
        { lineNumber: 1, value: { id: 1 } },
        { lineNumber: 2, value: { id: 2 } },
      ],
    });
    expect(parseStructuredPayload('{"id":1}\nnot json')).toEqual({ kind: "text" });
  });

  it("parses SSE blocks and JSON data fields", () => {
    expect(
      parseStructuredPayload(
        'event: response.output_item.done\ndata: {"type":"message"}\n\nid: 2\ndata: keepalive',
      ),
    ).toEqual({
      kind: "sse",
      events: [
        {
          sequence: 1,
          event: "response.output_item.done",
          id: null,
          retry: null,
          dataText: '{"type":"message"}',
          data: { type: "message" },
        },
        {
          sequence: 2,
          event: null,
          id: "2",
          retry: null,
          dataText: "keepalive",
          data: null,
        },
      ],
    });
  });

  it("preserves plain text and reports UTF-8 size", () => {
    expect(parseStructuredPayload("a very long plain text line")).toEqual({ kind: "text" });
    expect(getUtf8ByteLength("你好")).toBe(6);
  });
});
