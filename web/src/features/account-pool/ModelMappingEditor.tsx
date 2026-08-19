import {
  closestCenter,
  DndContext,
  type DragEndEvent,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  arrayMove,
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useCallback, useMemo } from "react";
import { Button } from "../../components/ui/button";
import { FilterableCombobox } from "../../components/ui/filterable-combobox";
import { Spinner } from "../../components/ui/spinner";
import { Switch } from "../../components/ui/switch";
import { useTranslation } from "../../i18n";
import type { ModelMapping } from "../../lib/api";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";

export const MAX_MODEL_MAPPINGS = 100;

export type ModelMappingDraft = ModelMapping & {
  id: string;
};

export function createModelMappingDrafts(
  mappings: readonly ModelMapping[] | null | undefined,
  createId: () => string,
): ModelMappingDraft[] {
  return (mappings ?? []).map((mapping) => ({
    id: createId(),
    sourceModel: mapping.sourceModel,
    targetModel: mapping.targetModel,
    enabled: mapping.enabled !== false,
  }));
}

export function toModelMappings(drafts: readonly ModelMappingDraft[]): ModelMapping[] {
  return drafts.map(({ sourceModel, targetModel, enabled }) => ({
    sourceModel: sourceModel.trim(),
    targetModel: targetModel.trim(),
    enabled,
  }));
}

export function areModelMappingDraftsEqual(
  left: readonly ModelMappingDraft[],
  right: readonly ModelMappingDraft[],
): boolean {
  if (left.length !== right.length) return false;
  return left.every((mapping, index) => {
    const candidate = right[index];
    return (
      candidate != null &&
      mapping.sourceModel.trim() === candidate.sourceModel.trim() &&
      mapping.targetModel.trim() === candidate.targetModel.trim() &&
      mapping.enabled === candidate.enabled
    );
  });
}

function asciiLowercase(value: string) {
  return value.replace(/[A-Z]/g, (character) => character.toLowerCase());
}

export type ModelMappingValidation = "empty" | "duplicate" | "limit" | null;

export function validateModelMappingDrafts(
  drafts: readonly ModelMappingDraft[],
): ModelMappingValidation {
  if (drafts.length > MAX_MODEL_MAPPINGS) return "limit";
  const sourceModels = new Set<string>();
  for (const mapping of drafts) {
    const sourceModel = mapping.sourceModel.trim();
    const targetModel = mapping.targetModel.trim();
    if (!sourceModel || !targetModel) return "empty";
    const sourceKey = asciiLowercase(sourceModel);
    if (sourceModels.has(sourceKey)) return "duplicate";
    sourceModels.add(sourceKey);
  }
  return null;
}

const inputClassName =
  "h-10 w-full rounded-lg border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60";

type ModelMappingEditorLabels = {
  enabled: string;
  remove: string;
  reorder: string;
  sourceModel: string;
  sourcePlaceholder: string;
  targetModel: string;
  targetPlaceholder: string;
  noSuggestions: string;
};

function SortableModelMappingRow({
  mapping,
  availableModelOptions,
  disabled,
  labels,
  onUpdate,
  onRemove,
}: {
  mapping: ModelMappingDraft;
  availableModelOptions: string[];
  disabled: boolean;
  labels: ModelMappingEditorLabels;
  onUpdate: (id: string, change: Partial<ModelMappingDraft>) => void;
  onRemove: (id: string) => void;
}) {
  const { attributes, isDragging, listeners, setNodeRef, transform, transition } = useSortable({
    id: mapping.id,
    disabled,
  });

  const dragHandleProps = { ...attributes, ...listeners };

  return (
    <div
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
      className={cn(
        "grid grid-cols-[minmax(0,1fr)_2.25rem] gap-3 rounded-lg border border-base-300/75 bg-base-100/55 p-3 transition-colors desktop:grid-cols-[2rem_minmax(0,1fr)_minmax(0,1fr)_auto_2.25rem] desktop:items-end",
        isDragging && "z-10 border-primary/55 bg-primary/5 shadow-md",
      )}
    >
      <button
        type="button"
        className="hidden h-9 w-9 touch-none cursor-grab items-center justify-center self-center rounded-md text-base-content/55 transition hover:bg-base-200 hover:text-base-content active:cursor-grabbing focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:pointer-events-none disabled:opacity-50 desktop:inline-flex"
        aria-label={labels.reorder}
        title={labels.reorder}
        disabled={disabled}
        {...dragHandleProps}
      >
        <AppIcon name="sort-variant" className="h-4 w-4" aria-hidden />
      </button>
      <label className="field col-span-2 min-w-0 gap-1.5 desktop:col-span-1">
        <span className="field-label desktop:sr-only">{labels.sourceModel}</span>
        <FilterableCombobox
          label={labels.sourceModel}
          name={`modelMappingSource-${mapping.id}`}
          value={mapping.sourceModel}
          onValueChange={(sourceModel) => onUpdate(mapping.id, { sourceModel })}
          options={availableModelOptions}
          placeholder={labels.sourcePlaceholder}
          emptyText={labels.noSuggestions}
          inputClassName={inputClassName}
          disabled={disabled}
        />
      </label>
      <label className="field col-span-2 min-w-0 gap-1.5 desktop:col-span-1">
        <span className="field-label desktop:sr-only">{labels.targetModel}</span>
        <FilterableCombobox
          label={labels.targetModel}
          name={`modelMappingTarget-${mapping.id}`}
          value={mapping.targetModel}
          onValueChange={(targetModel) => onUpdate(mapping.id, { targetModel })}
          options={availableModelOptions}
          placeholder={labels.targetPlaceholder}
          emptyText={labels.noSuggestions}
          inputClassName={inputClassName}
          disabled={disabled}
        />
      </label>
      <div className="col-span-2 flex min-h-10 items-center justify-between gap-3 desktop:col-span-1 desktop:justify-center">
        <span className="text-sm text-base-content/70 desktop:sr-only">{labels.enabled}</span>
        <Switch
          checked={mapping.enabled}
          onCheckedChange={(enabled) => onUpdate(mapping.id, { enabled })}
          aria-label={labels.enabled}
          disabled={disabled}
        />
      </div>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="hidden self-center text-base-content/65 hover:text-error desktop:inline-flex"
        aria-label={labels.remove}
        title={labels.remove}
        onClick={() => onRemove(mapping.id)}
        disabled={disabled}
      >
        <AppIcon name="delete-outline" className="h-4 w-4" aria-hidden />
      </Button>
      <div className="col-span-2 flex items-center justify-between border-t border-base-300/60 pt-2 desktop:hidden">
        <button
          type="button"
          className="inline-flex h-9 w-9 touch-none cursor-grab items-center justify-center rounded-md text-base-content/55 transition hover:bg-base-200 hover:text-base-content active:cursor-grabbing focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary disabled:pointer-events-none disabled:opacity-50"
          aria-label={labels.reorder}
          title={labels.reorder}
          disabled={disabled}
          {...dragHandleProps}
        >
          <AppIcon name="sort-variant" className="h-4 w-4" aria-hidden />
        </button>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="text-base-content/65 hover:text-error"
          aria-label={labels.remove}
          title={labels.remove}
          onClick={() => onRemove(mapping.id)}
          disabled={disabled}
        >
          <AppIcon name="delete-outline" className="h-4 w-4" aria-hidden />
        </Button>
      </div>
    </div>
  );
}

export function ModelMappingEditor({
  mappings,
  availableModelOptions,
  disabled = false,
  saving = false,
  dirty = false,
  saveError = null,
  onChange,
  onSave,
  createId,
}: {
  mappings: ModelMappingDraft[];
  availableModelOptions: string[];
  disabled?: boolean;
  saving?: boolean;
  dirty?: boolean;
  saveError?: string | null;
  onChange: (mappings: ModelMappingDraft[]) => void;
  onSave: () => void;
  createId: () => string;
}) {
  const { t } = useTranslation();
  const validation = useMemo(() => validateModelMappingDrafts(mappings), [mappings]);
  const canAdd = !disabled && !saving && mappings.length < MAX_MODEL_MAPPINGS;
  const mappingIds = useMemo(() => mappings.map((mapping) => mapping.id), [mappings]);
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 6 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const labels = useMemo<ModelMappingEditorLabels>(
    () => ({
      enabled: t("accountPool.upstreamAccounts.modelMappings.enabled"),
      remove: t("accountPool.upstreamAccounts.modelMappings.remove"),
      reorder: t("accountPool.upstreamAccounts.modelMappings.reorder"),
      sourceModel: t("accountPool.upstreamAccounts.modelMappings.sourceModel"),
      sourcePlaceholder: t("accountPool.upstreamAccounts.modelMappings.sourcePlaceholder"),
      targetModel: t("accountPool.upstreamAccounts.modelMappings.targetModel"),
      targetPlaceholder: t("accountPool.upstreamAccounts.modelMappings.targetPlaceholder"),
      noSuggestions: t("accountPool.upstreamAccounts.modelMappings.noSuggestions"),
    }),
    [t],
  );

  const validationMessage =
    validation == null
      ? null
      : t(`accountPool.upstreamAccounts.modelMappings.validation.${validation}`);

  const updateMapping = useCallback(
    (id: string, change: Partial<ModelMappingDraft>) => {
      onChange(
        mappings.map((mapping) => (mapping.id === id ? { ...mapping, ...change } : mapping)),
      );
    },
    [mappings, onChange],
  );

  const removeMapping = useCallback(
    (id: string) => {
      onChange(mappings.filter((mapping) => mapping.id !== id));
    },
    [mappings, onChange],
  );

  const handleDragEnd = useCallback(
    ({ active, over }: DragEndEvent) => {
      if (over == null || active.id === over.id) return;
      const sourceIndex = mappings.findIndex((mapping) => mapping.id === active.id);
      const targetIndex = mappings.findIndex((mapping) => mapping.id === over.id);
      if (sourceIndex < 0 || targetIndex < 0) return;
      onChange(arrayMove(mappings, sourceIndex, targetIndex));
    },
    [mappings, onChange],
  );

  return (
    <section
      className="border-t border-base-300/70 pt-5"
      aria-labelledby="model-mappings-heading"
      data-testid="model-mapping-editor"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <h3 id="model-mappings-heading" className="text-base font-semibold text-base-content">
            {t("accountPool.upstreamAccounts.modelMappings.title")}
          </h3>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() =>
            onChange([
              ...mappings,
              {
                id: createId(),
                sourceModel: "",
                targetModel: "",
                enabled: true,
              },
            ])
          }
          disabled={!canAdd}
        >
          <AppIcon name="plus" className="mr-2 h-4 w-4" aria-hidden />
          {t("accountPool.upstreamAccounts.modelMappings.add")}
        </Button>
      </div>

      {mappings.length > 0 ? (
        <div className="mt-4 space-y-2">
          <div className="hidden grid-cols-[2rem_minmax(0,1fr)_minmax(0,1fr)_auto_2.25rem] items-center gap-3 px-2 text-xs font-semibold uppercase text-base-content/52 desktop:grid">
            <span aria-hidden />
            <span>{t("accountPool.upstreamAccounts.modelMappings.sourceModel")}</span>
            <span>{t("accountPool.upstreamAccounts.modelMappings.targetModel")}</span>
            <span>{t("accountPool.upstreamAccounts.modelMappings.enabled")}</span>
            <span aria-hidden />
          </div>
          <DndContext
            collisionDetection={closestCenter}
            sensors={sensors}
            onDragEnd={handleDragEnd}
          >
            <SortableContext items={mappingIds} strategy={verticalListSortingStrategy}>
              {mappings.map((mapping) => (
                <SortableModelMappingRow
                  key={mapping.id}
                  mapping={mapping}
                  availableModelOptions={availableModelOptions}
                  disabled={disabled || saving}
                  labels={labels}
                  onUpdate={updateMapping}
                  onRemove={removeMapping}
                />
              ))}
            </SortableContext>
          </DndContext>
        </div>
      ) : (
        <p className="mt-4 border-y border-base-300/60 py-4 text-sm text-base-content/62">
          {t("accountPool.upstreamAccounts.modelMappings.empty")}
        </p>
      )}

      {saveError ? <p className="mt-3 text-sm text-error">{saveError}</p> : null}
      {validationMessage ? <p className="mt-3 text-sm text-error">{validationMessage}</p> : null}
      <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-base-300/60 pt-4">
        <span className="text-sm text-base-content/60">
          {t("accountPool.upstreamAccounts.modelMappings.count", {
            count: mappings.length,
          })}
        </span>
        <Button
          type="button"
          onClick={onSave}
          disabled={disabled || saving || validation != null || !dirty}
          data-testid="model-mapping-save"
        >
          {saving ? <Spinner size="sm" className="mr-2" /> : null}
          {saving
            ? t("accountPool.upstreamAccounts.modelMappings.saving")
            : t("accountPool.upstreamAccounts.modelMappings.save")}
        </Button>
      </div>
    </section>
  );
}
