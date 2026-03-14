import { Icon } from '@iconify/react'
import { Button } from './ui/button'
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog'

interface UpstreamAccountGroupNoteDialogProps {
  open: boolean
  groupName: string
  note: string
  busy?: boolean
  error?: string | null
  existing: boolean
  onNoteChange: (value: string) => void
  onClose: () => void
  onSave: () => void
  title: string
  existingDescription: string
  draftDescription: string
  noteLabel: string
  notePlaceholder: string
  cancelLabel: string
  saveLabel: string
  closeLabel: string
  existingBadgeLabel: string
  draftBadgeLabel: string
}

export function UpstreamAccountGroupNoteDialog({
  open,
  groupName,
  note,
  busy = false,
  error,
  existing,
  onNoteChange,
  onClose,
  onSave,
  title,
  existingDescription,
  draftDescription,
  noteLabel,
  notePlaceholder,
  cancelLabel,
  saveLabel,
  closeLabel,
  existingBadgeLabel,
  draftBadgeLabel,
}: UpstreamAccountGroupNoteDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy ? (nextOpen ? undefined : onClose()) : undefined)}>
      <DialogContent className="overflow-hidden border-base-300 bg-base-100 p-0">
        <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
          <DialogHeader className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <DialogTitle>{title}</DialogTitle>
              <span className="rounded-full border border-base-300/80 bg-base-200/80 px-2.5 py-1 text-xs font-semibold text-base-content/70">
                {existing ? existingBadgeLabel : draftBadgeLabel}
              </span>
            </div>
            <DialogDescription>
              {existing ? existingDescription : draftDescription}
            </DialogDescription>
            <p className="text-sm font-semibold text-base-content">{groupName}</p>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} disabled={busy} />
        </div>

        <div className="grid gap-4 px-6 py-5">
          {error ? (
            <div className="flex items-start gap-3 rounded-2xl border border-error/30 bg-error/10 px-4 py-3 text-sm text-error">
              <Icon icon="mdi:alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{error}</div>
            </div>
          ) : null}
          <label className="field">
            <span className="field-label">{noteLabel}</span>
            <textarea
              className="min-h-32 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
              value={note}
              placeholder={notePlaceholder}
              disabled={busy}
              onChange={(event) => onNoteChange(event.target.value)}
            />
          </label>
        </div>

        <DialogFooter className="border-t border-base-300/80 px-6 py-5">
          <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button type="button" onClick={onSave} disabled={busy}>
            {busy ? <Icon icon="mdi:loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden /> : null}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
