const encoder = new TextEncoder();

function measureUtf8Bytes(value: string) {
  return encoder.encode(value).length;
}

function toJson(value: unknown) {
  return JSON.stringify(value);
}

type SseBlock = {
  comment?: string;
  data?: string | Record<string, unknown>;
  event?: string;
  id?: string;
  retry?: number;
};

function buildSseBlock(block: SseBlock) {
  const lines: string[] = [];
  if (block.comment) lines.push(`: ${block.comment}`);
  if (block.event) lines.push(`event: ${block.event}`);
  if (block.id) lines.push(`id: ${block.id}`);
  if (typeof block.retry === "number") lines.push(`retry: ${block.retry}`);
  if (block.data !== undefined) {
    const dataText = typeof block.data === "string" ? block.data : toJson(block.data);
    for (const line of dataText.split("\n")) {
      lines.push(`data: ${line}`);
    }
  }
  return lines.join("\n");
}

function buildSseStream(blocks: SseBlock[]) {
  return `${blocks.map(buildSseBlock).join("\n\n")}\n\n`;
}

export const failedWorkflowAssistantText =
  "The attempt itself did not rewrite the upstream HTTP status. " +
  "The upstream accepted the request, started a text/event-stream response, and emitted tool " +
  "activity plus assistant output before the downstream side closed. " +
  "The caller-facing HTTP 502 was produced later by final adjudication, not by the attempt " +
  "record itself. " +
  "For debugging, the owner should focus on routeMode=pool, endpoint=/v1/responses, " +
  "account=pool-alpha@example.com, requestedServiceTier=priority, responseModel=gpt-5.4, " +
  "request compression=gzip, response compression=gzip, and the downstream_closed terminal " +
  "failure during streaming. " +
  "Because the raw stream already contained completed tool activity and assistant text, this " +
  "attempt should be treated as partial upstream work that became unusable only when the " +
  "system performed the final caller-facing adjudication after downstream termination.";

export const failedWorkflowRequestBody = {
  model: "gpt-5.4",
  stream: true,
  service_tier: "priority",
  max_output_tokens: 1800,
  parallel_tool_calls: true,
  tool_choice: "auto",
  reasoning: {
    effort: "high",
    summary: "auto",
  },
  text: {
    format: {
      type: "text",
    },
  },
  include: [
    "reasoning.encrypted_content",
    "web_search_call.action.sources",
    "function_call.arguments",
  ],
  instructions:
    "You are a diagnostics assistant for invocation workflow investigations. " +
    "Explain attempt-level behavior and final caller-facing adjudication separately. " +
    "Never imply that an attempt rewrote the upstream HTTP status. " +
    "When the raw stream already contains assistant text or tool activity, preserve that " +
    "fact even if the final system result becomes a failure. " +
    "Prioritize route selection, upstream account choice, service tier, headers, body " +
    "capture availability, tool calls, and the exact chain from streaming progress to " +
    "downstream termination. " +
    "Keep the answer compact enough for an observability drawer, but concrete enough that an " +
    "operator can tell whether the problem came from request construction, upstream delivery, " +
    "stream parsing, or final adjudication.",
  tools: [
    {
      type: "function",
      name: "search_docs",
      description:
        "Retrieve internal troubleshooting material for proxy routing, streaming failures, " +
        "response capture, final adjudication, request/response body retention, and owner " +
        "facing diagnostics surfaces.",
      parameters: {
        type: "object",
        properties: {
          topic: {
            type: "string",
            description: "Main failure topic such as downstream_closed or upstream_stream_error.",
          },
          surface: {
            type: "string",
            description: "UI surface being debugged, for example invocation-workflow-detail.",
          },
          include_sections: {
            type: "array",
            items: {
              type: "string",
            },
            description:
              "Concrete sections to retrieve, for example routing, streaming, compression, " +
              "body_capture, or final_adjudication.",
          },
          attempt_id: {
            type: "string",
            description: "Attempt public id or local attempt identifier when available.",
          },
          caller_expectation: {
            type: "string",
            description:
              "What the operator is trying to verify, such as whether the attempt mutated " +
              "the upstream status or whether the response had already completed upstream.",
          },
        },
        required: ["topic", "surface", "include_sections"],
        additionalProperties: false,
      },
    },
    {
      type: "web_search_preview",
      user_location: {
        type: "approximate",
        country: "US",
        city: "San Francisco",
        region: "California",
      },
      search_context_size: "medium",
    },
  ],
  input: [
    {
      role: "user",
      content: [
        {
          type: "input_text",
          text:
            "The owner is inspecting invocation invoke-workflow-77. " +
            "There is one attempt, the request went to /v1/responses through pool routing, " +
            "the upstream account was pool-alpha@example.com, the route binding was " +
            "fpb_tokyo_alpha, and the final caller-facing result shown in the workflow panel " +
            "is HTTP 502 service_failure. " +
            "The attempt card itself shows upstream HTTP 200, streaming phase, response model " +
            "gpt-5.4, and downstream_closed. " +
            "Explain what that combination means without introducing invented stages or hiding " +
            "the distinction between raw transport facts and system adjudication.",
        },
      ],
    },
    {
      role: "user",
      content: [
        {
          type: "input_text",
          text:
            "The detail view must tell the operator whether the request body and response body " +
            "were retained in full, whether compression was applied on the wire, which tools " +
            "had already been called upstream before the stream ended for the caller, and " +
            "which fields come from parsed business semantics versus raw capture. " +
            "Avoid filler text. Use wording that can be rendered directly inside a compact " +
            "workflow detail panel.",
        },
      ],
    },
  ],
  metadata: {
    invoke_id: "invoke-workflow-77",
    panel: "invocation-workflow",
    viewer: "owner",
  },
} as const;

export const failedWorkflowRequestBodyText = toJson(failedWorkflowRequestBody);

export const failedWorkflowSearchItem = {
  id: "ws_77",
  type: "web_search_call",
  status: "completed",
  query: "downstream closed while streaming upstream response final adjudication",
  action: {
    type: "search",
    sources: [
      {
        title: "Downstream closed while streaming upstream response",
        url: "https://internal.example/search/downstream-closed",
      },
      {
        title: "Final adjudication rules for invocation workflows",
        url: "https://internal.example/search/final-adjudication",
      },
    ],
  },
} as const;

export const failedWorkflowFunctionArguments = toJson({
  topic: "downstream_closed",
  surface: "invocation-workflow-detail",
  include_sections: ["routing", "streaming", "compression", "body_capture", "final_adjudication"],
  attempt_id: "attempt-1",
  caller_expectation: "show why upstream HTTP 200 can coexist with final HTTP 502",
});

export const failedWorkflowFunctionItem = {
  id: "fc_77",
  type: "function_call",
  status: "completed",
  name: "search_docs",
  arguments: failedWorkflowFunctionArguments,
  call_id: "call_search_docs_77",
} as const;

export const failedWorkflowMessageItem = {
  id: "msg_77",
  type: "message",
  status: "completed",
  role: "assistant",
  content: [
    {
      type: "output_text",
      text: failedWorkflowAssistantText,
    },
  ],
} as const;

export const failedWorkflowResponseBody = {
  id: "resp_77",
  object: "response",
  status: "completed",
  model: "gpt-5.4",
  service_tier: "priority",
  output: [failedWorkflowSearchItem, failedWorkflowFunctionItem, failedWorkflowMessageItem],
  usage: {
    input_tokens: 148,
    output_tokens: 92,
    total_tokens: 240,
  },
} as const;

const failedWorkflowResponseSseBlocks: SseBlock[] = [
  {
    comment: "keepalive",
  },
  {
    event: "response.created",
    id: "evt_77_1",
    data: {
      type: "response.created",
      response: {
        id: failedWorkflowResponseBody.id,
        object: failedWorkflowResponseBody.object,
        model: failedWorkflowResponseBody.model,
        status: "in_progress",
        service_tier: failedWorkflowResponseBody.service_tier,
      },
    },
  },
  {
    event: "response.in_progress",
    id: "evt_77_2",
    data: {
      type: "response.in_progress",
      response: {
        id: failedWorkflowResponseBody.id,
        model: failedWorkflowResponseBody.model,
        status: "in_progress",
        service_tier: failedWorkflowResponseBody.service_tier,
      },
    },
  },
  {
    event: "response.output_item.added",
    id: "evt_77_3",
    data: {
      type: "response.output_item.added",
      output_index: 0,
      item: {
        id: failedWorkflowSearchItem.id,
        type: failedWorkflowSearchItem.type,
        status: "in_progress",
        query: failedWorkflowSearchItem.query,
      },
    },
  },
  {
    event: "response.output_item.done",
    id: "evt_77_4",
    data: {
      type: "response.output_item.done",
      output_index: 0,
      item: failedWorkflowSearchItem,
    },
  },
  {
    event: "response.output_item.added",
    id: "evt_77_5",
    data: {
      type: "response.output_item.added",
      output_index: 1,
      item: {
        id: failedWorkflowFunctionItem.id,
        type: failedWorkflowFunctionItem.type,
        status: "in_progress",
        name: failedWorkflowFunctionItem.name,
        call_id: failedWorkflowFunctionItem.call_id,
      },
    },
  },
  {
    event: "response.function_call_arguments.delta",
    id: "evt_77_6",
    data: {
      type: "response.function_call_arguments.delta",
      item_id: failedWorkflowFunctionItem.id,
      output_index: 1,
      delta: failedWorkflowFunctionArguments.slice(0, 124),
    },
  },
  {
    event: "response.function_call_arguments.delta",
    id: "evt_77_7",
    data: {
      type: "response.function_call_arguments.delta",
      item_id: failedWorkflowFunctionItem.id,
      output_index: 1,
      delta: failedWorkflowFunctionArguments.slice(124),
    },
  },
  {
    event: "response.output_item.done",
    id: "evt_77_8",
    data: {
      type: "response.output_item.done",
      output_index: 1,
      item: failedWorkflowFunctionItem,
    },
  },
  {
    event: "response.output_item.added",
    id: "evt_77_9",
    data: {
      type: "response.output_item.added",
      output_index: 2,
      item: {
        id: failedWorkflowMessageItem.id,
        type: failedWorkflowMessageItem.type,
        status: "in_progress",
        role: failedWorkflowMessageItem.role,
      },
    },
  },
  {
    event: "response.output_text.delta",
    id: "evt_77_10",
    data: {
      type: "response.output_text.delta",
      item_id: failedWorkflowMessageItem.id,
      output_index: 2,
      content_index: 0,
      delta: failedWorkflowAssistantText.slice(0, 260),
    },
  },
  {
    event: "response.output_text.delta",
    id: "evt_77_11",
    data: {
      type: "response.output_text.delta",
      item_id: failedWorkflowMessageItem.id,
      output_index: 2,
      content_index: 0,
      delta: failedWorkflowAssistantText.slice(260, 520),
    },
  },
  {
    event: "response.output_text.delta",
    id: "evt_77_12",
    data: {
      type: "response.output_text.delta",
      item_id: failedWorkflowMessageItem.id,
      output_index: 2,
      content_index: 0,
      delta: failedWorkflowAssistantText.slice(520),
    },
  },
  {
    event: "response.output_item.done",
    id: "evt_77_13",
    data: {
      type: "response.output_item.done",
      output_index: 2,
      item: failedWorkflowMessageItem,
    },
  },
  {
    event: "response.completed",
    id: "evt_77_14",
    data: {
      type: "response.completed",
      response: failedWorkflowResponseBody,
    },
  },
];

export const failedWorkflowResponseBodyText = buildSseStream(failedWorkflowResponseSseBlocks);

export const failedWorkflowFinalResponseBody = {
  error: {
    message: "downstream closed while streaming",
    type: "service_failure",
    code: "downstream_closed",
  },
  invoke_id: "invoke-workflow-77",
  status: 502,
} as const;

export const failedWorkflowFinalResponseBodyText = toJson(failedWorkflowFinalResponseBody);

export const failedWorkflowRequestBodySize = measureUtf8Bytes(failedWorkflowRequestBodyText);
export const failedWorkflowResponseBodySize = measureUtf8Bytes(failedWorkflowResponseBodyText);
