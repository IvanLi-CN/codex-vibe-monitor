import { cn } from "../../lib/utils";
import { AnimatedDigits } from "../shared/AnimatedDigits";
import { AppIcon } from "../shared/AppIcon";
import { formatDashboardNetworkSpeed } from "./dashboardNetworkFormatting";

export function DashboardNetworkSpeedCapsule({
  uploadBytesPerSecond,
  downloadBytesPerSecond,
  localeTag,
  uploadLabel,
  downloadLabel,
  testId,
  className,
}: {
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  localeTag: string;
  uploadLabel: string;
  downloadLabel: string;
  testId?: string;
  className?: string;
}) {
  const uploadValue = formatDashboardNetworkSpeed(uploadBytesPerSecond, localeTag);
  const downloadValue = formatDashboardNetworkSpeed(downloadBytesPerSecond, localeTag);

  return (
    <div
      data-testid={testId}
      className={cn(
        "inline-flex min-w-0 max-w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-full border border-base-300/65 bg-base-100/78 px-2.5 py-1",
        className,
      )}
    >
      <span className="inline-flex min-w-0 items-center gap-1 whitespace-nowrap text-sky-500 dark:text-sky-300">
        <AppIcon name="arrow-up-bold" className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="font-mono text-[0.82rem] font-semibold leading-none">
          <span className="sr-only">{uploadLabel}: </span>
          <AnimatedDigits value={uploadValue} />
        </span>
      </span>
      <span className="inline-flex min-w-0 items-center gap-1 whitespace-nowrap text-emerald-500 dark:text-emerald-300">
        <AppIcon name="arrow-down-bold" className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="font-mono text-[0.82rem] font-semibold leading-none">
          <span className="sr-only">{downloadLabel}: </span>
          <AnimatedDigits value={downloadValue} />
        </span>
      </span>
    </div>
  );
}
