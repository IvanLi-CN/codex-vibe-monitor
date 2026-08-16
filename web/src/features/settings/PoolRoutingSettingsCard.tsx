import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import { Chip } from "../../components/ui/chip";
import { Input } from "../../components/ui/input";
import { SelectField } from "../../components/ui/select-field";
import { Switch } from "../../components/ui/switch";
import { useTranslation } from "../../i18n";
import type {
  AvailableModelsMode,
  CodexImagegenRewriteMode,
  RequestCompressionAlgorithm,
  RequestCompressionLevelPreset,
} from "../../lib/api";
import {
  requestCompressionAlgorithmLabel,
  requestCompressionLevelPresetLabel,
} from "../../lib/requestCompression";
import {
  MultiSelectFilterCombobox,
  type MultiSelectFilterOption,
} from "../account-pool/MultiSelectFilterCombobox";
import { PolicyInlineOptionGroup } from "../account-pool/PolicyInlineOptionGroup";
import { AppIcon } from "../shared/AppIcon";

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
    codexImagegenRewriteMode: CodexImagegenRewriteMode;
    availableModels: string[];
    availableModelsMode: AvailableModelsMode;
    responsesFirstByteTimeoutSecs: string;
    compactFirstByteTimeoutSecs: string;
    imageFirstByteTimeoutSecs: string;
    responsesStreamTimeoutSecs: string;
    compactStreamTimeoutSecs: string;
    cacheHitProtectionEnabled: boolean;
    cacheHitRateThresholdPercent: string;
    cacheHitOverflowMode: "queue" | "reroute";
    liveRequestStreamingEnabled: boolean;
    liveRequestStreamingGroupNames: string;
    liveRequestStreamingTreatmentPercent: string;
  };
  busy: boolean;
  writesEnabled: boolean;
  canSave: boolean;
  validationMessage?: string | null;
  onAlgorithmChange: (value: RequestCompressionAlgorithm) => void;
  onLevelPresetChange: (value: RequestCompressionLevelPreset) => void;
  onCodexImagegenRewriteModeChange: (value: CodexImagegenRewriteMode) => void;
  availableModelOptions: string[];
  onAvailableModelsChange: (value: string[]) => void;
  onAvailableModelsModeChange: (value: AvailableModelsMode) => void;
  onTimeoutChange: (key: RoutingTimeoutFieldKey, value: string) => void;
  onCacheHitProtectionChange: (patch: {
    cacheHitProtectionEnabled?: boolean;
    cacheHitRateThresholdPercent?: string;
    cacheHitOverflowMode?: "queue" | "reroute";
  }) => void;
  onLiveRequestStreamingChange: (patch: {
    liveRequestStreamingEnabled?: boolean;
    liveRequestStreamingGroupNames?: string;
    liveRequestStreamingTreatmentPercent?: string;
  }) => void;
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
  onCodexImagegenRewriteModeChange,
  availableModelOptions,
  onAvailableModelsChange,
  onAvailableModelsModeChange,
  onTimeoutChange,
  onCacheHitProtectionChange,
  onLiveRequestStreamingChange,
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
  const availableModelComboboxOptions: MultiSelectFilterOption[] = Array.from(
    new Set([...availableModelOptions, ...draft.availableModels]),
  ).map((value) => ({
    value,
    label: value.startsWith("gpt-image") ? `Image · ${value}` : value,
  }));
  const statusChipText = !writesEnabled
    ? t("settings.routing.readOnly")
    : busy
      ? t("settings.saving")
      : canSave
        ? t("settings.routing.unsaved")
        : t("settings.routing.saved");
  const statusChipTone = !writesEnabled ? "secondary" : canSave ? "warning" : "success";
  const availableModelsModeLabel =
    draft.availableModelsMode === "allowlist"
      ? t("settings.routing.models.allowlist")
      : t("settings.routing.models.denylist");
  const nextAvailableModelsMode =
    draft.availableModelsMode === "allowlist" ? "denylist" : "allowlist";
  const nextAvailableModelsModeLabel =
    nextAvailableModelsMode === "allowlist"
      ? t("settings.routing.models.allowlist")
      : t("settings.routing.models.denylist");
  const availableModelsModeToggleLabel = t("settings.routing.models.toggle", {
    current: availableModelsModeLabel,
    next: nextAvailableModelsModeLabel,
  });

  return (
    <Card className="mobile-flat-surface overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
      <CardHeader className="mobile-flat-surface-header gap-3 border-b border-base-300/70 pb-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="space-y-1.5">
            <CardTitle>{t("settings.routing.title")}</CardTitle>
            <CardDescription>{t("settings.routing.description")}</CardDescription>
          </div>
          <Chip tone={statusChipTone} className="shrink-0">
            {statusChipText}
          </Chip>
        </div>
      </CardHeader>

      <CardContent className="mobile-flat-surface-body space-y-5 pt-4">
        <div className="space-y-4 rounded-lg border border-base-300/75 bg-base-200/28 p-4">
          <div className="flex items-start justify-between gap-4">
            <div className="space-y-1">
              <div className="font-medium leading-snug">{t("settings.routing.cacheHit.title")}</div>
              <div className="text-sm leading-snug text-base-content/70">
                {t("settings.routing.cacheHit.description", { tokens: 3840 })}
              </div>
            </div>
            <Switch
              checked={draft.cacheHitProtectionEnabled}
              disabled={!writesEnabled || busy}
              aria-label={t("settings.routing.cacheHit.title")}
              onCheckedChange={(checked) =>
                onCacheHitProtectionChange({ cacheHitProtectionEnabled: checked })
              }
            />
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="field">
              <span className="field-label">{t("settings.routing.cacheHit.threshold")}</span>
              <Input
                type="number"
                min="1"
                max="100"
                inputMode="numeric"
                value={draft.cacheHitRateThresholdPercent}
                disabled={!writesEnabled || busy || !draft.cacheHitProtectionEnabled}
                onChange={(event) =>
                  onCacheHitProtectionChange({
                    cacheHitRateThresholdPercent: event.target.value,
                  })
                }
              />
            </label>
            <SelectField
              className="field"
              label={t("settings.routing.cacheHit.overflow")}
              name="settingsRoutingCacheHitOverflow"
              value={draft.cacheHitOverflowMode}
              disabled={!writesEnabled || busy || !draft.cacheHitProtectionEnabled}
              options={[
                { value: "queue", label: t("settings.routing.cacheHit.queue") },
                { value: "reroute", label: t("settings.routing.cacheHit.reroute") },
              ]}
              onValueChange={(value) =>
                onCacheHitProtectionChange({
                  cacheHitOverflowMode: value as "queue" | "reroute",
                })
              }
            />
          </div>
        </div>
        <div className="space-y-4 rounded-lg border border-base-300/75 bg-base-200/28 p-4">
          <div className="flex items-start justify-between gap-4">
            <div className="space-y-1">
              <div className="font-medium leading-snug">
                {t("settings.routing.liveRequestStreaming.title")}
              </div>
              <div className="text-sm leading-snug text-base-content/70">
                {t("settings.routing.liveRequestStreaming.description")}
              </div>
            </div>
            <Switch
              checked={draft.liveRequestStreamingEnabled}
              disabled={!writesEnabled || busy}
              aria-label={t("settings.routing.liveRequestStreaming.title")}
              onCheckedChange={(checked) =>
                onLiveRequestStreamingChange({ liveRequestStreamingEnabled: checked })
              }
            />
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="field">
              <span className="field-label">
                {t("settings.routing.liveRequestStreaming.groupNames")}
              </span>
              <Input
                value={draft.liveRequestStreamingGroupNames}
                disabled={!writesEnabled || busy || !draft.liveRequestStreamingEnabled}
                onChange={(event) =>
                  onLiveRequestStreamingChange({
                    liveRequestStreamingGroupNames: event.target.value,
                  })
                }
              />
            </label>
            <label className="field">
              <span className="field-label">
                {t("settings.routing.liveRequestStreaming.treatmentPercent")}
              </span>
              <Input
                type="number"
                min="0"
                max="100"
                inputMode="numeric"
                value={draft.liveRequestStreamingTreatmentPercent}
                disabled={!writesEnabled || busy || !draft.liveRequestStreamingEnabled}
                onChange={(event) =>
                  onLiveRequestStreamingChange({
                    liveRequestStreamingTreatmentPercent: event.target.value,
                  })
                }
              />
            </label>
          </div>
          <p className="text-xs leading-snug text-base-content/60">
            {t("settings.routing.liveRequestStreaming.hint")}
          </p>
        </div>
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
              {t("settings.routing.codexImagegen.title")}
            </div>
            <div className="text-sm leading-snug text-base-content/70">
              {t("settings.routing.codexImagegen.description")}
            </div>
          </div>
          <SelectField
            className="field"
            label={t("settings.routing.codexImagegen.mode")}
            name="settingsRoutingCodexImagegenRewriteMode"
            value={draft.codexImagegenRewriteMode}
            disabled={!writesEnabled || busy}
            options={[
              { value: "keep_original", label: t("settings.routing.codexImagegen.keepOriginal") },
              { value: "fill_missing", label: t("settings.routing.codexImagegen.fillMissing") },
              { value: "force_add", label: t("settings.routing.codexImagegen.forceAdd") },
              { value: "force_remove", label: t("settings.routing.codexImagegen.forceRemove") },
            ]}
            onValueChange={(value) =>
              onCodexImagegenRewriteModeChange(value as CodexImagegenRewriteMode)
            }
          />
        </div>

        <div className="space-y-3 rounded-xl border border-base-300/75 bg-base-200/28 p-4">
          <div className="space-y-1">
            <div className="font-medium leading-snug">{t("settings.routing.models.title")}</div>
            <div className="text-sm leading-snug text-base-content/70">
              {t("settings.routing.models.description")}
            </div>
          </div>
          <div className="flex flex-col gap-3 min-[769px]:flex-row min-[769px]:items-center">
            <div className="w-full min-[769px]:w-auto min-[769px]:shrink-0">
              <Button
                type="button"
                variant="outline"
                aria-pressed={draft.availableModelsMode === "allowlist"}
                aria-label={availableModelsModeToggleLabel}
                title={availableModelsModeToggleLabel}
                disabled={!writesEnabled || busy}
                data-mode={draft.availableModelsMode}
                onClick={() => onAvailableModelsModeChange(nextAvailableModelsMode)}
                className="hidden h-9 w-auto gap-1.5 rounded-md px-3 min-[769px]:inline-flex"
              >
                <span>{availableModelsModeLabel}</span>
                <AppIcon name="compare-horizontal" className="h-4 w-4" aria-hidden />
              </Button>
              <div className="min-[769px]:hidden">
                <PolicyInlineOptionGroup<AvailableModelsMode>
                  ariaLabel={t("settings.routing.models.mode")}
                  value={draft.availableModelsMode}
                  disabled={!writesEnabled || busy}
                  options={[
                    { value: "allowlist", label: t("settings.routing.models.allowlist") },
                    { value: "denylist", label: t("settings.routing.models.denylist") },
                  ]}
                  onChange={onAvailableModelsModeChange}
                />
              </div>
            </div>
            <div className="min-w-0 flex-1">
              <MultiSelectFilterCombobox
                options={availableModelComboboxOptions}
                value={draft.availableModels}
                onValueChange={onAvailableModelsChange}
                disabled={!writesEnabled || busy}
                placeholder={t("settings.routing.models.empty")}
                searchPlaceholder={t("settings.routing.models.search")}
                emptyLabel={t("settings.routing.models.empty")}
                clearLabel={t("settings.routing.models.clear")}
                ariaLabel={t("settings.routing.models.title")}
              />
            </div>
          </div>
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
                  className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 font-mono text-sm"
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
