import { afterEach, describe, expect, it, vi } from 'vitest'
import { copyText, selectAllReadonlyText } from './clipboard'

function createMockDocument(result: boolean) {
  const textarea = {
    value: '',
    style: {} as Record<string, string>,
    contentEditable: 'inherit',
    readOnly: true,
    focus: vi.fn(),
    select: vi.fn(),
    setSelectionRange: vi.fn(),
    setAttribute: vi.fn(),
  }
  const body = {
    appendChild: vi.fn(),
    removeChild: vi.fn(),
  }
  const execCommand = vi.fn(() => result)
  const doc = {
    body,
    activeElement: null,
    createElement: vi.fn(() => textarea),
    execCommand,
    getSelection: vi.fn(() => null),
  } as unknown as Document

  return { doc, textarea, body, execCommand }
}

describe('copyText', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('prefers the Clipboard API when available', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    const result = await copyText('https://example.com/oauth', {
      nav: {
        clipboard: { writeText },
      } as unknown as Navigator,
      allowExecCommand: false,
    })

    expect(result).toEqual({
      ok: true,
      method: 'clipboard',
    })
    expect(writeText).toHaveBeenCalledWith('https://example.com/oauth')
  })

  it('falls back to execCommand when the Clipboard API rejects', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('blocked'))
    const { doc, textarea, body, execCommand } = createMockDocument(true)

    const result = await copyText('https://example.com/oauth', {
      doc,
      nav: {
        clipboard: { writeText },
        userAgent: 'Mozilla/5.0',
        platform: 'MacIntel',
        maxTouchPoints: 0,
      } as unknown as Navigator,
    })

    expect(result.ok).toBe(true)
    expect(result.method).toBe('execCommand')
    expect(writeText).toHaveBeenCalledTimes(1)
    expect(execCommand).toHaveBeenCalledWith('copy')
    expect(body.appendChild).toHaveBeenCalledWith(textarea)
    expect(body.removeChild).toHaveBeenCalledWith(textarea)
  })

  it('returns a failure result when every copy path is blocked', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('blocked'))
    const { doc, execCommand } = createMockDocument(false)

    const result = await copyText('https://example.com/oauth', {
      doc,
      nav: {
        clipboard: { writeText },
        userAgent: 'Mozilla/5.0',
        platform: 'MacIntel',
        maxTouchPoints: 0,
      } as unknown as Navigator,
    })

    expect(result.ok).toBe(false)
    expect(result.method).toBeNull()
    expect(result.errors?.clipboard).toBeInstanceOf(Error)
    expect(result.errors?.execCommand).toBeInstanceOf(Error)
    expect(execCommand).toHaveBeenCalledWith('copy')
  })
})

describe('selectAllReadonlyText', () => {
  it('focuses and selects the entire readonly field', () => {
    const target = {
      value: 'manual-copy-target',
      focus: vi.fn(),
      select: vi.fn(),
      setSelectionRange: vi.fn(),
    }

    selectAllReadonlyText(target)

    expect(target.focus).toHaveBeenCalledTimes(1)
    expect(target.select).toHaveBeenCalledTimes(1)
    expect(target.setSelectionRange).toHaveBeenCalledWith(0, target.value.length)
  })
})
