import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { TranslationKey } from "../../i18n";
import type { ApiPoolUpstreamRequestAttempt } from "../../lib/api";
import { PoolAttemptRecordCard } from "./PoolAttemptRecordCard";

const labels: Partial<Record<TranslationKey, string>> = {
  "table.poolAttempts.status.success": "Success",
  "table.poolAttempts.retry": "Retry",
  "table.poolAttempts.proxy": "Proxy",
  "table.poolAttempts.upstreamHttpStatus": "Upstream HTTP status",
  "table.poolAttempts.downstreamHttpStatus": "Downstream HTTP status",
  "table.poolAttempts.failureKind": "Failure kind",
  "table.poolAttempts.originalModel": "Original model",
  "table.poolAttempts.upstreamRequestModel": "Upstream model",
  "table.poolAttempts.modelMappingPattern": "Matched mapping",
  "table.poolAttempts.modelNotSent": "Not sent",
  "table.poolAttempts.connectLatency": "Connect latency",
  "table.poolAttempts.firstByteLatency": "First-byte latency",
  "table.poolAttempts.streamLatency": "Stream latency",
  "table.poolAttempts.startedAt": "Started at",
  "table.poolAttempts.finishedAt": "Finished at",
  "table.poolAttempts.upstreamRequestId": "Upstream request ID",
};

const t = (key: TranslationKey) => labels[key] ?? key;

const baseAttempt: ApiPoolUpstreamRequestAttempt = {
  attemptId: "attempt-mapping",
  invokeId: "invoke-mapping",
  occurredAt: "2026-08-19T09:00:00Z",
  endpoint: "/v1/responses",
  upstreamAccountId: 5,
  upstreamAccountName: "Mapping account",
  requestModel: "client-fast",
  model: "client-fast",
  attemptIndex: 1,
  distinctAccountIndex: 1,
  sameAccountRetryIndex: 0,
  status: "success",
  createdAt: "2026-08-19T09:00:00Z",
};

function renderAttempt(attempt: ApiPoolUpstreamRequestAttempt) {
  return renderToStaticMarkup(
    <PoolAttemptRecordCard
      attempt={attempt}
      proxyDisplay={{ value: "Direct", title: "Direct", resolved: true }}
      t={t}
    />,
  );
}

describe("PoolAttemptRecordCard model mapping audit", () => {
  it("shows the client model, actual upstream target, and matched source rule", () => {
    const markup = renderAttempt({
      ...baseAttempt,
      upstreamRequestModel: "upstream-fast",
      modelMappingPattern: "client-*",
    });

    expect(markup).toContain("Original model");
    expect(markup).toContain(">client-fast<");
    expect(markup).toContain("Upstream model");
    expect(markup).toContain(">upstream-fast<");
    expect(markup).toContain("Matched mapping");
    expect(markup).toContain(">client-*<");
  });

  it("falls back to the original model for historical records and labels a mapped pre-send failure", () => {
    expect(renderAttempt(baseAttempt)).toContain(">client-fast<");
    expect(
      renderAttempt({
        ...baseAttempt,
        upstreamRequestModel: null,
        modelMappingPattern: "client-*",
      }),
    ).toContain(">Not sent<");
  });
});
