import type { ApiInvocation } from "../lib/api";
import { cn } from "../lib/utils";

function isWebSocketInvocation(record: Pick<ApiInvocation, "transport">) {
  return record.transport?.trim().toLowerCase() === "websocket";
}

export function renderInvocationTransportBadge(
  record: Pick<ApiInvocation, "transport">,
  className?: string,
) {
  if (!isWebSocketInvocation(record)) return null;

  return (
    <span
      aria-label="WebSocket"
      title="WebSocket"
      data-testid="invocation-transport-badge"
      className={cn(
        "inline-flex h-4 shrink-0 items-center rounded-full border border-primary/45 bg-primary/10 px-1.5 py-0 text-[8px] font-semibold leading-none text-primary shadow-none",
        className,
      )}
    >
      WS
    </span>
  );
}
