export const STRUCTURED_PAYLOAD_AUTO_PARSE_LIMIT_BYTES = 1024 * 1024;

export type StructuredPayloadValue = Record<string, unknown> | unknown[];

export interface ParsedSseEvent {
  sequence: number;
  event: string | null;
  id: string | null;
  retry: string | null;
  dataText: string;
  data: StructuredPayloadValue | null;
}

export type ParsedStructuredPayload =
  | { kind: "json"; value: StructuredPayloadValue }
  | { kind: "ndjson"; values: Array<{ lineNumber: number; value: StructuredPayloadValue }> }
  | { kind: "sse"; events: ParsedSseEvent[] }
  | { kind: "text" };

function asStructuredValue(value: unknown): StructuredPayloadValue | null {
  if (Array.isArray(value)) return value;
  if (value != null && typeof value === "object") {
    return value as Record<string, unknown>;
  }
  return null;
}

function parseStructuredJson(value: string): StructuredPayloadValue | null {
  try {
    return asStructuredValue(JSON.parse(value));
  } catch {
    return null;
  }
}

function parseSseEvents(value: string): ParsedSseEvent[] | null {
  const normalized = value.replace(/\r\n/g, "\n");
  const blocks = normalized.split(/\n{2,}/).filter((block) => block.trim().length > 0);
  if (blocks.length === 0) return null;

  let recognizedFieldCount = 0;
  const events = blocks
    .map((block) => {
      let event: string | null = null;
      let id: string | null = null;
      let retry: string | null = null;
      const dataLines: string[] = [];

      for (const line of block.split("\n")) {
        if (!line || line.startsWith(":")) continue;
        const separatorIndex = line.indexOf(":");
        const field = separatorIndex >= 0 ? line.slice(0, separatorIndex) : line;
        const rawValue = separatorIndex >= 0 ? line.slice(separatorIndex + 1) : "";
        const fieldValue = rawValue.startsWith(" ") ? rawValue.slice(1) : rawValue;

        if (field === "event") {
          recognizedFieldCount += 1;
          event = fieldValue;
        } else if (field === "id") {
          recognizedFieldCount += 1;
          id = fieldValue;
        } else if (field === "retry") {
          recognizedFieldCount += 1;
          retry = fieldValue;
        } else if (field === "data") {
          recognizedFieldCount += 1;
          dataLines.push(fieldValue);
        }
      }

      const dataText = dataLines.join("\n");
      return {
        sequence: 0,
        event,
        id,
        retry,
        dataText,
        data: dataText ? parseStructuredJson(dataText) : null,
      };
    })
    .filter((entry) => entry.event || entry.id || entry.retry || entry.dataText)
    .map((entry, index) => ({ ...entry, sequence: index + 1 }));

  if (recognizedFieldCount === 0 || !events.some((event) => event.dataText || event.event)) {
    return null;
  }
  return events;
}

export function getUtf8ByteLength(value: string) {
  return new TextEncoder().encode(value).byteLength;
}

export function parseStructuredPayload(value: string): ParsedStructuredPayload {
  const trimmed = value.trim();
  if (!trimmed) return { kind: "text" };

  const json = parseStructuredJson(trimmed);
  if (json) return { kind: "json", value: json };

  const nonEmptyLines = value.split(/\r?\n/).filter((line) => line.trim().length > 0);
  if (nonEmptyLines.length > 1) {
    const values = nonEmptyLines.map((line, index) => ({
      lineNumber: index + 1,
      value: parseStructuredJson(line.trim()),
    }));
    if (
      values.every(
        (entry): entry is { lineNumber: number; value: StructuredPayloadValue } =>
          entry.value != null,
      )
    ) {
      return { kind: "ndjson", values };
    }
  }

  const events = parseSseEvents(value);
  if (events) return { kind: "sse", events };

  return { kind: "text" };
}
