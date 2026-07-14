import { useCallback, useMemo, useState } from "react";
import { defaultStyles, JsonView } from "react-json-view-lite";
import "react-json-view-lite/dist/index.css";
import { Button } from "../../components/ui/button";
import { cn } from "../../lib/utils";
import {
  getUtf8ByteLength,
  parseStructuredPayload,
  STRUCTURED_PAYLOAD_AUTO_PARSE_LIMIT_BYTES,
  type StructuredPayloadValue,
} from "./structuredPayload";

interface StructuredPayloadViewerProps {
  value: string;
  labels: {
    json: string;
    ndjson: string;
    sse: string;
    text: string;
    largePayload: string;
    parseLargePayload: string;
    event: string;
    data: string;
    expand: string;
    collapse: string;
  };
  className?: string;
}

const jsonStyles = {
  ...defaultStyles,
  container: "structured-payload-json",
  basicChildStyle: "structured-payload-json-child",
  childFieldsContainer: "structured-payload-json-children",
  label: "structured-payload-json-key",
  clickableLabel: "structured-payload-json-key structured-payload-json-clickable",
  nullValue: "structured-payload-json-null",
  undefinedValue: "structured-payload-json-null",
  stringValue: "structured-payload-json-string",
  booleanValue: "structured-payload-json-boolean",
  numberValue: "structured-payload-json-number",
  otherValue: "structured-payload-json-other",
  punctuation: "structured-payload-json-punctuation",
  collapseIcon: "structured-payload-json-collapse",
  expandIcon: "structured-payload-json-expand",
  collapsedContent: "structured-payload-json-collapsed",
  ariaLables: {
    collapseJson: "Collapse JSON",
    expandJson: "Expand JSON",
  },
};

function RawText({ value }: { value: string }) {
  return (
    <pre className="structured-payload-raw" data-testid="structured-payload-raw">
      {value}
    </pre>
  );
}

function JsonTree({
  value,
  expandDepth,
  labels,
}: {
  value: StructuredPayloadValue;
  expandDepth: number;
  labels: StructuredPayloadViewerProps["labels"];
}) {
  const shouldExpandNode = useCallback((level: number) => level < expandDepth, [expandDepth]);
  const styles = useMemo(
    () => ({
      ...jsonStyles,
      ariaLables: {
        collapseJson: labels.collapse,
        expandJson: labels.expand,
      },
    }),
    [labels.collapse, labels.expand],
  );
  return (
    <JsonView
      data={value}
      style={styles}
      shouldExpandNode={shouldExpandNode}
      clickToExpandNode
      aria-label={labels.json}
    />
  );
}

export function StructuredPayloadViewer({
  value,
  labels,
  className,
}: StructuredPayloadViewerProps) {
  const byteLength = useMemo(() => getUtf8ByteLength(value), [value]);
  const isLarge = byteLength > STRUCTURED_PAYLOAD_AUTO_PARSE_LIMIT_BYTES;
  const [parseLargePayload, setParseLargePayload] = useState(false);
  const parsed = useMemo(
    () => (isLarge && !parseLargePayload ? null : parseStructuredPayload(value)),
    [isLarge, parseLargePayload, value],
  );

  if (parsed == null) {
    return (
      <div className={cn("min-w-0 max-w-full space-y-2", className)}>
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-warning/25 bg-warning/8 px-3 py-2">
          <span className="text-xs leading-5 text-base-content/72">{labels.largePayload}</span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => setParseLargePayload(true)}
          >
            {labels.parseLargePayload}
          </Button>
        </div>
        <RawText value={value} />
      </div>
    );
  }

  if (parsed.kind === "text") {
    return (
      <div className={cn("min-w-0 max-w-full", className)} data-payload-kind="text">
        <RawText value={value} />
      </div>
    );
  }

  return (
    <div
      className={cn("min-w-0 max-w-full space-y-2", className)}
      data-testid="structured-payload-viewer"
      data-payload-kind={parsed.kind}
    >
      <span className="sr-only">{value}</span>
      <div className="text-[11px] font-semibold text-base-content/62">
        {parsed.kind === "json"
          ? labels.json
          : parsed.kind === "ndjson"
            ? labels.ndjson
            : labels.sse}
      </div>
      <div className="structured-payload-scroll">
        {parsed.kind === "json" ? (
          <JsonTree
            value={parsed.value}
            expandDepth={byteLength < 64 * 1024 ? 2 : 1}
            labels={labels}
          />
        ) : parsed.kind === "ndjson" ? (
          <div className="space-y-2">
            {parsed.values.map((entry) => (
              <section className="structured-payload-entry" key={entry.lineNumber}>
                <div className="structured-payload-entry-label">#{entry.lineNumber}</div>
                <JsonTree value={entry.value} expandDepth={1} labels={labels} />
              </section>
            ))}
          </div>
        ) : (
          <div className="space-y-2">
            {parsed.events.map((entry) => (
              <section className="structured-payload-entry" key={entry.sequence}>
                <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs">
                  <span className="font-semibold text-base-content/74">
                    {labels.event} #{entry.sequence}
                  </span>
                  {entry.event ? (
                    <code className="break-all text-primary">{entry.event}</code>
                  ) : null}
                  {entry.id ? (
                    <code className="break-all text-base-content/62">id: {entry.id}</code>
                  ) : null}
                  {entry.retry ? (
                    <code className="break-all text-base-content/62">retry: {entry.retry}</code>
                  ) : null}
                </div>
                {entry.data ? (
                  <JsonTree value={entry.data} expandDepth={1} labels={labels} />
                ) : entry.dataText ? (
                  <div className="mt-2">
                    <div className="structured-payload-entry-label">{labels.data}</div>
                    <RawText value={entry.dataText} />
                  </div>
                ) : null}
              </section>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
