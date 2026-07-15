import type { RequestCompressionAlgorithm, RequestCompressionLevelPreset } from "./api";

export const REQUEST_COMPRESSION_INHERIT_VALUE = "__inherit__";

export interface RequestCompressionAlgorithmLabels {
  requestCompressionFollow: string;
  requestCompressionIdentity: string;
  requestCompressionGzip: string;
  requestCompressionDeflate: string;
  requestCompressionZstd: string;
  requestCompressionInherited?: string;
}

export interface RequestCompressionLevelPresetLabels {
  requestCompressionLevelFast: string;
  requestCompressionLevelBalanced: string;
  requestCompressionLevelBest: string;
}

export interface RequestCompressionModeLabels {
  requestCompressionModeIdentity: string;
  requestCompressionModePassthrough: string;
  requestCompressionModeRecompressed: string;
}

export function requestCompressionAlgorithmLabel(
  value: RequestCompressionAlgorithm | typeof REQUEST_COMPRESSION_INHERIT_VALUE | null | undefined,
  labels: RequestCompressionAlgorithmLabels,
): string {
  switch (value) {
    case REQUEST_COMPRESSION_INHERIT_VALUE:
      return labels.requestCompressionInherited ?? "Inherit";
    case "follow":
      return labels.requestCompressionFollow;
    case "gzip":
      return labels.requestCompressionGzip;
    case "deflate":
      return labels.requestCompressionDeflate;
    case "zstd":
      return labels.requestCompressionZstd;
    case "identity":
    default:
      return labels.requestCompressionIdentity;
  }
}

export function requestCompressionLevelPresetLabel(
  value: RequestCompressionLevelPreset | null | undefined,
  labels: RequestCompressionLevelPresetLabels,
): string {
  switch (value) {
    case "fast":
      return labels.requestCompressionLevelFast;
    case "best":
      return labels.requestCompressionLevelBest;
    case "balanced":
    default:
      return labels.requestCompressionLevelBalanced;
  }
}

export function requestCompressionModeLabel(
  value: string | null | undefined,
  labels: RequestCompressionModeLabels,
): string {
  switch (value) {
    case "passthrough":
      return labels.requestCompressionModePassthrough;
    case "recompressed":
      return labels.requestCompressionModeRecompressed;
    case "identity":
    default:
      return labels.requestCompressionModeIdentity;
  }
}
