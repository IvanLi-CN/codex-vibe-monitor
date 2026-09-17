import { expect, it } from "vitest";
import { resolveInvocationEndpointDisplay } from "../../lib/invocation";

it("maps the recognized invocation endpoints onto badge metadata", () => {
  expect(resolveInvocationEndpointDisplay({ endpoint: " /v1/responses " })).toEqual({
    kind: "responses",
    endpointValue: "/v1/responses",
    chipTone: "blue",
    labelKey: "table.endpoint.responsesBadge",
  });
  expect(resolveInvocationEndpointDisplay("/v1/chat/completions")).toEqual({
    kind: "chat",
    endpointValue: "/v1/chat/completions",
    chipTone: "teal",
    labelKey: "table.endpoint.chatBadge",
  });
  expect(resolveInvocationEndpointDisplay("/v1/responses/compact")).toEqual({
    kind: "compact",
    endpointValue: "/v1/responses/compact",
    chipTone: "orange",
    labelKey: "table.endpoint.compactBadge",
  });
  expect(resolveInvocationEndpointDisplay("/v1/images/generations")).toEqual({
    kind: "image_gen",
    endpointValue: "/v1/images/generations",
    chipTone: "emerald",
    labelKey: "table.endpoint.imageGenBadge",
  });
  expect(resolveInvocationEndpointDisplay("/v1/images/edits")).toEqual({
    kind: "image_edit",
    endpointValue: "/v1/images/edits",
    chipTone: "amber",
    labelKey: "table.endpoint.imageEditBadge",
  });
  expect(resolveInvocationEndpointDisplay("/v1/images/variations")).toEqual({
    kind: "image",
    endpointValue: "/v1/images/variations",
    chipTone: "cyan",
    labelKey: "table.endpoint.imageBadge",
  });
});
it("surfaces remote compaction v2 only while in flight or when response compaction fired", () => {
  expect(
    resolveInvocationEndpointDisplay({
      endpoint: "/v1/responses",
      status: "running",
      compactionRequestKind: "remote_v2",
    }),
  ).toEqual({
    kind: "remote_v2",
    endpointValue: "/v1/responses",
    chipTone: "violet",
    labelKey: "table.endpoint.remoteV2Badge",
  });

  expect(
    resolveInvocationEndpointDisplay({
      endpoint: "/v1/responses",
      status: "success",
      compactionRequestKind: "remote_v2",
      compactionResponseKind: null,
    }),
  ).toEqual({
    kind: "responses",
    endpointValue: "/v1/responses",
    chipTone: "blue",
    labelKey: "table.endpoint.responsesBadge",
  });

  expect(
    resolveInvocationEndpointDisplay({
      endpoint: "/v1/responses",
      status: "success",
      compactionResponseKind: "remote_v2",
    }),
  ).toEqual({
    kind: "remote_v2",
    endpointValue: "/v1/responses",
    chipTone: "violet",
    labelKey: "table.endpoint.remoteV2Badge",
  });
});
it("keeps unknown or missing endpoints on the raw fallback path", () => {
  expect(resolveInvocationEndpointDisplay("/v1/responses/experimental")).toEqual({
    kind: "raw",
    endpointValue: "/v1/responses/experimental",
    chipTone: null,
    labelKey: null,
  });
  expect(resolveInvocationEndpointDisplay(undefined)).toEqual({
    kind: "raw",
    endpointValue: "—",
    chipTone: null,
    labelKey: null,
  });
});
