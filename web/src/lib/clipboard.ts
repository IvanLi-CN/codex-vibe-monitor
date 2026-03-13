export type CopyMethod = 'clipboard' | 'execCommand'

export interface CopyTextResult {
  ok: boolean
  method: CopyMethod | null
  errors?: {
    clipboard?: unknown
    execCommand?: unknown
  }
}

export interface CopyTextOptions {
  doc?: Document
  nav?: Navigator
  allowExecCommand?: boolean
  preferExecCommand?: boolean
}

export interface SelectAllTextTarget {
  value: string
  focus?: () => void
  select: () => void
  setSelectionRange?: (start: number, end: number) => void
}

export async function copyText(value: string, options: CopyTextOptions = {}): Promise<CopyTextResult> {
  const nav = options.nav ?? (typeof navigator !== 'undefined' ? navigator : undefined)
  const doc = options.doc ?? (typeof document !== 'undefined' ? document : undefined)
  const allowExecCommand = options.allowExecCommand ?? true
  const preferExecCommand = options.preferExecCommand ?? false

  let execCommandError: unknown
  const tryExecCommand = () => {
    if (!allowExecCommand) return false
    try {
      return copyTextWithExecCommand(value, doc, nav)
    } catch (error) {
      execCommandError = error
      return false
    }
  }

  if (preferExecCommand && tryExecCommand()) {
    return { ok: true, method: 'execCommand' }
  }

  let clipboardError: unknown
  if (nav?.clipboard?.writeText) {
    try {
      await nav.clipboard.writeText(value)
      return { ok: true, method: 'clipboard' }
    } catch (error) {
      clipboardError = error
    }
  }

  if (!preferExecCommand && tryExecCommand()) {
    return {
      ok: true,
      method: 'execCommand',
      errors: clipboardError ? { clipboard: clipboardError } : undefined,
    }
  }

  return {
    ok: false,
    method: null,
    errors: {
      clipboard: clipboardError,
      execCommand: execCommandError,
    },
  }
}

export function copyTextWithExecCommand(value: string, doc?: Document, nav?: Navigator): boolean {
  if (!doc?.body) {
    throw new Error('Document body is unavailable for clipboard fallback')
  }

  const activeElement = typeof HTMLElement !== 'undefined' && doc.activeElement instanceof HTMLElement
    ? doc.activeElement
    : null
  const selection = doc.getSelection?.() ?? null
  const ranges = selection
    ? Array.from({ length: selection.rangeCount }, (_, index) => selection.getRangeAt(index).cloneRange())
    : []

  const textarea = doc.createElement('textarea')
  textarea.value = value
  textarea.setAttribute('readonly', 'true')
  textarea.setAttribute('aria-hidden', 'true')
  textarea.style.position = 'fixed'
  textarea.style.top = '0'
  textarea.style.left = '-9999px'
  textarea.style.opacity = '0'
  textarea.style.pointerEvents = 'none'
  textarea.style.whiteSpace = 'pre'

  doc.body.appendChild(textarea)
  let restoreExecCommandTarget: (() => void) | void = undefined

  try {
    restoreExecCommandTarget = selectExecCommandTarget(textarea, doc, nav)
    const copied = doc.execCommand('copy')
    if (!copied) {
      throw new Error('document.execCommand("copy") returned false')
    }
    return true
  } finally {
    restoreExecCommandTarget?.()
    doc.body.removeChild(textarea)
    if (selection) {
      selection.removeAllRanges()
      for (const range of ranges) {
        selection.addRange(range)
      }
    }
    activeElement?.focus()
  }
}

export function selectAllReadonlyText(target: SelectAllTextTarget | null | undefined): void {
  if (!target) return
  target.focus?.()
  target.select()
  target.setSelectionRange?.(0, target.value.length)
}

function selectExecCommandTarget(target: HTMLTextAreaElement, doc: Document, nav?: Navigator): (() => void) | void {
  if (requiresIOSSelectionHack(nav)) {
    const selection = doc.getSelection?.() ?? null
    const range = doc.createRange()
    const originalContentEditable = target.contentEditable
    const originalReadOnly = target.readOnly

    target.contentEditable = 'true'
    target.readOnly = false
    target.focus()
    range.selectNodeContents(target)
    selection?.removeAllRanges()
    selection?.addRange(range)
    target.setSelectionRange(0, target.value.length)
    return () => {
      target.contentEditable = originalContentEditable
      target.readOnly = originalReadOnly
    }
  }

  selectAllReadonlyText(target)
}

function requiresIOSSelectionHack(nav?: Navigator): boolean {
  if (!nav) return false
  const userAgent = nav.userAgent ?? ''
  const platform = nav.platform ?? ''
  const maxTouchPoints = typeof nav.maxTouchPoints === 'number' ? nav.maxTouchPoints : 0

  if (/iPad|iPhone|iPod/i.test(userAgent)) {
    return true
  }

  return platform === 'MacIntel' && maxTouchPoints > 1
}
