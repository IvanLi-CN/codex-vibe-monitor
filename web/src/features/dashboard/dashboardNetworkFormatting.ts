const KIB = 1024;
const MIB = KIB * 1024;

function formatScaledNumber(
  value: number,
  localeTag: string,
  minimumFractionDigits: number,
  maximumFractionDigits: number,
) {
  return new Intl.NumberFormat(localeTag, {
    minimumFractionDigits,
    maximumFractionDigits,
  }).format(value);
}

export function formatDashboardNetworkBytes(value: number, localeTag: string) {
  const safeValue = Number.isFinite(value) ? Math.max(0, value) : 0;
  if (safeValue >= MIB) {
    return `${formatScaledNumber(safeValue / MIB, localeTag, 0, 1)} MiB`;
  }
  if (safeValue >= KIB) {
    return `${formatScaledNumber(safeValue / KIB, localeTag, 0, 1)} KiB`;
  }
  return `${formatScaledNumber(safeValue, localeTag, 0, 0)} B`;
}

export function formatDashboardNetworkSpeed(value: number, localeTag: string) {
  const safeValue = Number.isFinite(value) ? Math.max(0, value) : 0;
  if (safeValue >= MIB) {
    return `${formatScaledNumber(safeValue / MIB, localeTag, 0, 1)} MiB/s`;
  }
  if (safeValue >= KIB) {
    return `${formatScaledNumber(safeValue / KIB, localeTag, 0, 1)} KiB/s`;
  }
  return `${formatScaledNumber(safeValue, localeTag, 0, 0)} B/s`;
}
