import { Chip } from "../../components/ui/chip";
import { useTranslation } from "../../i18n";
import type { ApiInvocation } from "../../lib/api";

function isWebSocketInvocation(record: Pick<ApiInvocation, "transport">) {
  return record.transport?.trim().toLowerCase() === "websocket";
}

export function renderInvocationTransportChip(
  record: Pick<ApiInvocation, "transport">,
  className?: string,
) {
  if (!isWebSocketInvocation(record)) return null;

  return <InvocationTransportChip className={className} />;
}

function InvocationTransportChip({ className }: { className?: string }) {
  const { t } = useTranslation();
  const historicalLabel = t("records.filters.transport.websocket");

  return (
    <Chip
      tone="primary"
      size="micro"
      title={historicalLabel}
      data-testid="invocation-transport-badge"
      className={className}
    >
      <span>{historicalLabel}</span>
    </Chip>
  );
}
