import { useState } from "react";
import { Button } from "../../components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import type {
  EffectiveRoutingTimeoutFieldSources,
  GroupAccountRoutingRule,
  PoolRoutingTimeoutSettings,
  UpdateGroupAccountRoutingRulePayload,
} from "../../lib/api";
import type { GroupAccountRoutingRuleLabels } from "./GroupAccountRoutingRuleDialog.model";
import { GroupAccountRoutingRuleEditorContent } from "./GroupAccountRoutingRuleEditorContent";
import { useGroupAccountRoutingRuleEditorState } from "./GroupAccountRoutingRuleEditorState";

export type { GroupAccountRoutingRuleLabels } from "./GroupAccountRoutingRuleDialog.model";

interface GroupAccountRoutingRuleEditorProps {
  open: boolean;
  rule?: GroupAccountRoutingRule | null;
  busy?: boolean;
  error?: string | null;
  changedFieldsOnly?: boolean;
  effectiveTimeouts?: PoolRoutingTimeoutSettings | null;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources | null;
  timeoutOverrideSource?: "group" | "account";
  labels: GroupAccountRoutingRuleLabels;
  availableModelOptions?: string[];
  className?: string;
  onPayloadChange?: (payload: UpdateGroupAccountRoutingRulePayload | null) => void;
}

interface GroupAccountRoutingRuleDialogProps
  extends Omit<GroupAccountRoutingRuleEditorProps, "className" | "onPayloadChange"> {
  title: string;
  description: string;
  submitLabel: string;
  onClose: () => void;
  onSubmit: (payload: UpdateGroupAccountRoutingRulePayload) => Promise<void> | void;
}

export function GroupAccountRoutingRuleEditor({
  open,
  rule,
  busy = false,
  error,
  changedFieldsOnly = false,
  effectiveTimeouts,
  timeoutFieldSources,
  timeoutOverrideSource = "group",
  labels,
  availableModelOptions = [],
  className,
  onPayloadChange,
}: GroupAccountRoutingRuleEditorProps) {
  const { draft, setDraft, payload, timeoutValidationError } =
    useGroupAccountRoutingRuleEditorState({
      open,
      rule,
      changedFieldsOnly,
      effectiveTimeouts,
      timeoutFieldSources,
      timeoutOverrideSource,
      labels,
      onPayloadChange,
    });

  return (
    <GroupAccountRoutingRuleEditorContent
      className={className}
      draft={draft}
      setDraft={setDraft}
      busy={busy}
      changedFieldsOnly={changedFieldsOnly}
      effectiveTimeouts={effectiveTimeouts}
      timeoutFieldSources={timeoutFieldSources}
      labels={labels}
      availableModelOptions={availableModelOptions}
      error={error}
      timeoutValidationError={timeoutValidationError}
      payload={payload}
    />
  );
}

export function GroupAccountRoutingRuleDialog({
  open,
  title,
  description,
  submitLabel,
  rule,
  busy = false,
  error,
  changedFieldsOnly = false,
  effectiveTimeouts,
  timeoutFieldSources,
  timeoutOverrideSource = "group",
  onClose,
  onSubmit,
  labels,
  availableModelOptions = [],
}: GroupAccountRoutingRuleDialogProps) {
  const [payload, setPayload] = useState<UpdateGroupAccountRoutingRulePayload | null>(null);
  const disabled = !payload || busy;

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy && !nextOpen ? onClose() : undefined)}>
      <DialogContent className="flex max-h-[calc(100dvh-0.75rem)] max-w-none flex-col overflow-hidden p-0 desktop:max-h-[min(90vh,calc(100vh-2rem))] desktop:w-[min(48rem,calc(100vw-2rem))]">
        <div className="shrink-0 border-b border-base-300/80 px-5 py-4 desktop:px-6 desktop:py-5">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 desktop:px-6">
          <GroupAccountRoutingRuleEditor
            open={open}
            rule={rule}
            busy={busy}
            error={error}
            changedFieldsOnly={changedFieldsOnly}
            effectiveTimeouts={effectiveTimeouts}
            timeoutFieldSources={timeoutFieldSources}
            timeoutOverrideSource={timeoutOverrideSource}
            labels={labels}
            availableModelOptions={availableModelOptions}
            onPayloadChange={setPayload}
          />
        </div>
        <div className="shrink-0 border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:px-6 desktop:py-4">
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
              {labels.cancel}
            </Button>
            <Button
              type="button"
              disabled={disabled}
              onClick={() => {
                if (payload) void onSubmit(payload);
              }}
            >
              {submitLabel}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
