import { Chip } from "../../components/ui/chip";
import type { ApiInvocation } from "../../lib/api";

function isWebSocketInvocation(record: Pick<ApiInvocation, "transport">) {
  return record.transport?.trim().toLowerCase() === "websocket";
}

export function renderInvocationTransportChip(
  record: Pick<ApiInvocation, "transport">,
  className?: string,
) {
  if (!isWebSocketInvocation(record)) return null;

  return (
    <Chip
      tone="primary"
      size="micro"
      title="WebSocket"
      data-testid="invocation-transport-badge"
      className={className}
    >
      <span aria-hidden="true">WS</span>
      <span className="sr-only">WebSocket transport</span>
    </Chip>
  );
}
