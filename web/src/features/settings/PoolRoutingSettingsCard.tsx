import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { useTranslation } from "../../i18n";
import type { RequestCompressionAlgorithm, RequestCompressionLevelPreset } from "../../lib/api";
import {
  requestCompressionAlgorithmLabel,
  requestCompressionLevelPresetLabel,
} from "../../lib/requestCompression";

type RoutingTimeoutFieldKey =
  | "responsesFirstByteTimeoutSecs"
  | "compactFirstByteTimeoutSecs"
  | "imageFirstByteTimeoutSecs"
  | "responsesStreamTimeoutSecs"
  | "compactStreamTimeoutSecs";

type PoolRoutingSettingsCardProps = {
  draft: {
    requestCompressionAlgorithm: RequestCompressionAlgorithm;
    requestCompressionLevelPreset: RequestCompressionLevelPreset;
    responsesFirstByteTimeoutSecs: string;
    compactFirstByteTimeoutSecs: string;
    imageFirstByteTimeoutSecs: string;
    responsesStreamTimeoutSecs: string;
    compactStreamTimeoutSecs: string;
  };
  busy: boolean;
  writesEnabled: boolean;
  canSave: boolean;
  validationMessage?: string | null;
  onAlgorithmChange: (value: RequestCompressionAlgorithm) => void;
  onLevelPresetChange: (value: RequestCompressionLevelPreset) => void;
  onTimeoutChange: (key: RoutingTimeoutFieldKey, value: string) => void;
  onSave: () => void;
};

export function PoolRoutingSettingsCard({
  draft,
  busy,
  writesEnabled,
  canSave,
  validationMessage,
  onAlgorithmChange,
  onLevelPresetChange,
  onTimeoutChange,
  onSave,
}: PoolRoutingSettingsCardProps) {
  const { t } = useTranslation();
  const compressionLabelMap = {
    requestCompressionFollow: t("accountPool.requestCompression.follow"),
    requestCompressionIdentity: t("accountPool.requestCompression.identity"),
    requestCompressionGzip: t("accountPool.requestCompression.gzip"),
    requestCompressionDeflate: t("accountPool.requestCompression.deflate"),
    requestCompressionZstd: t("accountPool.requestCompression.zstd"),
  };
  const levelLabelMap = {
    requestCompressionLevelFast: t("accountPool.requestCompression.level.fast"),
    requestCompressionLevelBalanced: t("accountPool.requestCompression.level.balanced"),
    requestCompressionLevelBest: t("accountPool.requestCompression.level.best"),
  };
  const timeoutFields: Array<{ key: RoutingTimeoutFieldKey; label: string; value: string }> = [
    {
      key: "responsesFirstByteTimeoutSecs",
      label: t("settings.routing.timeout.responsesFirstByte"),
      value: draft.responsesFirstByteTimeoutSecs,
    },
    {
      key: "compactFirstByteTimeoutSecs",
      label: t("settings.routing.timeout.compactFirstByte"),
      value: draft.compactFirstByteTimeoutSecs,
    },
    {
      key: "imageFirstByteTimeoutSecs",
      label: t("settings.routing.timeout.imageFirstByte"),
      value: draft.imageFirstByteTimeoutSecs,
    },
    {
      key: "responsesStreamTimeoutSecs",
      label: t("settings.routing.timeout.responsesStream"),
      value: draft.responsesStreamTimeoutSecs,
    },
    {
      key: "compactStreamTimeoutSecs",
      label: t("settings.routing.timeout.compactStream"),
      value: draft.compactStreamTimeoutSecs,
    },
  ];
  const statusBadgeText = !writesEnabled
    ? t("settings.routing.readOnly")
    : busy
      ? t("settings.saving")
      : canSave
        ? t("settings.routing.unsaved")
        : t("settings.routing.saved");
  const statusBadgeVariant = !writesEnabled ? "secondary" : canSave ? "warning" : "success";

  return (
    <Card className="mobile-flat-surface overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
      <CardHeader className="mobile-flat-surface-header gap-3 border-b border-base-300/70 pb-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="space-y-1.5">
            <CardTitle>{t("settings.routing.title")}</CardTitle>
            <CardDescription>{t("settings.routing.description")}</CardDescription>
          </div>
          <Badge variant={statusBadgeVariant} className="shrink-0">
            {statusBadgeText}
          </Badge>
        </div>
      </CardHeader>

      <CardContent className="mobile-flat-surface-body space-y-5 pt-4">
        <div className="space-y-4 rounded-xl border border-base-300/75 bg-base-200/28 p-4">
          <div className="space-y-1">
            <div className="font-medium leading-snug">
              {t("settings.routing.requestCompressionSectionTitle")}
            </div>
            <div className="text-sm leading-snug text-base-content/70">
              {t("settings.routing.requestCompressionSectionDescription")}
            </div>
          </div>

          <div className="grid gap-3 xl:grid-cols-2">
            <SelectField
              className="field"
              label={t("settings.routing.requestCompressionAlgorithmLabel")}
              name="settingsRoutingRequestCompressionAlgorithm"
              value={draft.requestCompressionAlgorithm}
              disabled={!writesEnabled || busy}
              options={[
                {
                  value: "follow",
                  label: requestCompressionAlgorithmLabel("follow", compressionLabelMap),
                },
                {
                  value: "identity",
                  label: requestCompressionAlgorithmLabel("identity", compressionLabelMap),
                },
                {
                  value: "gzip",
                  label: requestCompressionAlgorithmLabel("gzip", compressionLabelMap),
                },
                {
                  value: "deflate",
                  label: requestCompressionAlgorithmLabel("deflate", compressionLabelMap),
                },
                {
                  value: "zstd",
                  label: requestCompressionAlgorithmLabel("zstd", compressionLabelMap),
                },
              ]}
              onValueChange={(value) => onAlgorithmChange(value as RequestCompressionAlgorithm)}
            />

            <SelectField
              className="field"
              label={t("settings.routing.requestCompressionLevelPresetLabel")}
              name="settingsRoutingRequestCompressionLevelPreset"
              value={draft.requestCompressionLevelPreset}
              disabled={!writesEnabled || busy}
              options={[
                {
                  value: "fast",
                  label: requestCompressionLevelPresetLabel("fast", levelLabelMap),
                },
                {
                  value: "balanced",
                  label: requestCompressionLevelPresetLabel("balanced", levelLabelMap),
                },
                {
                  value: "best",
                  label: requestCompressionLevelPresetLabel("best", levelLabelMap),
                },
              ]}
              onValueChange={(value) => onLevelPresetChange(value as RequestCompressionLevelPreset)}
            />
          </div>

          <p className="text-xs leading-snug text-base-content/60">
            {t("settings.routing.requestCompressionHint")}
          </p>
        </div>

        <div className="space-y-3 rounded-xl border border-base-300/75 bg-base-200/28 p-4">
          <div className="space-y-1">
            <div className="font-medium leading-snug">
              {t("settings.routing.timeout.sectionTitle")}
            </div>
            <div className="text-sm leading-snug text-base-content/70">
              {t("settings.routing.timeout.sectionDescription")}
            </div>
          </div>

          <div className="grid gap-3 xl:grid-cols-2">
            {timeoutFields.map((field) => (
              <label key={field.key} className="field">
                <span className="field-label">{field.label}</span>
                <Input
                  name={field.key}
                  type="number"
                  min="1"
                  step="1"
                  value={field.value}
                  disabled={!writesEnabled || busy}
                  className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                  onChange={(event) => onTimeoutChange(field.key, event.target.value)}
                />
              </label>
            ))}
          </div>
        </div>

        {validationMessage ? (
          <Alert variant="error" className="text-sm">
            {validationMessage}
          </Alert>
        ) : null}

        <div className="flex justify-end">
          <Button type="button" disabled={!writesEnabled || busy || !canSave} onClick={onSave}>
            {busy ? t("settings.saving") : t("settings.routing.save")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
