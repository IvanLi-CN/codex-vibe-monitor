import type { ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Stories } from '@storybook/addon-docs/blocks'
import { expect, within } from 'storybook/test'
import { cn } from '../../lib/utils'
import { FloatingFieldBubble } from './floating-field-bubble'
import { Input } from './input'
import type { BubbleVariant } from './bubble'

function expectBubbleArrowInsideFlatSegment(bubble: Element | null) {
  expect(bubble).toBeTruthy()

  const bubbleElement = bubble as HTMLElement
  const arrow = bubbleElement.querySelector('[data-bubble-arrow="true"]')

  expect(arrow).toBeTruthy()

  const bubbleRect = bubbleElement.getBoundingClientRect()
  const arrowRect = (arrow as HTMLElement).getBoundingClientRect()
  const radius = Number.parseFloat(getComputedStyle(bubbleElement).borderTopLeftRadius || '0')
  const side = bubbleElement.getAttribute('data-side')

  if (side === 'left' || side === 'right') {
    const flatTop = bubbleRect.top + radius
    const flatBottom = bubbleRect.bottom - radius

    expect(arrowRect.top >= flatTop - 1).toBe(true)
    expect(arrowRect.bottom <= flatBottom + 1).toBe(true)
    if (side === 'left') {
      expect(arrowRect.left - bubbleRect.right <= 0.5).toBe(true)
    } else {
      expect(bubbleRect.left - arrowRect.right <= 0.5).toBe(true)
    }
    return
  }

  if (side === 'top' || side === 'bottom') {
    const flatLeft = bubbleRect.left + radius
    const flatRight = bubbleRect.right - radius

    expect(arrowRect.left >= flatLeft - 1).toBe(true)
    expect(arrowRect.right <= flatRight + 1).toBe(true)
    if (side === 'bottom') {
      expect(bubbleRect.top - arrowRect.bottom <= 0.5).toBe(true)
    } else {
      expect(arrowRect.top - bubbleRect.bottom <= 0.5).toBe(true)
    }
  }
}

function backdropImage(theme: 'vibe-light' | 'vibe-dark') {
  const palette = theme === 'vibe-light'
    ? {
        base: '#eef4ff',
        panel: '#f8fbff',
        line: 'rgba(139, 157, 189, 0.26)',
        orbA: 'rgba(124, 58, 237, 0.12)',
        orbB: 'rgba(16, 185, 129, 0.10)',
        orbC: 'rgba(56, 189, 248, 0.10)',
      }
    : {
        base: '#1a2231',
        panel: '#202b3d',
        line: 'rgba(166, 180, 204, 0.16)',
        orbA: 'rgba(56, 189, 248, 0.12)',
        orbB: 'rgba(52, 211, 153, 0.10)',
        orbC: 'rgba(129, 140, 248, 0.12)',
      }

  const svg = `
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 820" preserveAspectRatio="none">
      <rect width="1200" height="820" fill="${palette.base}"/>
      <rect x="28" y="28" width="1144" height="764" rx="40" fill="${palette.panel}" fill-opacity="0.76"/>
      <g stroke="${palette.line}" stroke-width="1">
        <path d="M0 190H1200"/>
        <path d="M0 410H1200"/>
        <path d="M0 630H1200"/>
        <path d="M180 0V820"/>
        <path d="M540 0V820"/>
        <path d="M900 0V820"/>
      </g>
      <circle cx="1030" cy="130" r="170" fill="${palette.orbA}"/>
      <circle cx="180" cy="720" r="210" fill="${palette.orbB}"/>
      <circle cx="760" cy="540" r="120" fill="${palette.orbC}"/>
    </svg>
  `

  return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`
}

function motionPalette(theme: 'vibe-light' | 'vibe-dark') {
  return theme === 'vibe-light'
    ? {
        mist: 'rgba(255,255,255,0.58)',
        beam: 'rgba(255,255,255,0.42)',
        orbA: 'rgba(99,102,241,0.22)',
        orbB: 'rgba(45,212,191,0.18)',
        orbC: 'rgba(56,189,248,0.14)',
        ripple: 'rgba(255,255,255,0.34)',
      }
    : {
        mist: 'rgba(255,255,255,0.08)',
        beam: 'rgba(96,165,250,0.16)',
        orbA: 'rgba(59,130,246,0.22)',
        orbB: 'rgba(16,185,129,0.16)',
        orbC: 'rgba(129,140,248,0.18)',
        ripple: 'rgba(148,163,184,0.12)',
      }
}

function StoryMotionStyles() {
  return (
    <style>
      {`
        @keyframes bubble-aurora-float {
          0% { transform: translate3d(-6%, -4%, 0) scale(0.96); opacity: 0.42; }
          50% { transform: translate3d(7%, 4%, 0) scale(1.05); opacity: 0.72; }
          100% { transform: translate3d(14%, -3%, 0) scale(1.1); opacity: 0.54; }
        }

        @keyframes bubble-aurora-drift {
          0% { transform: translate3d(0, 0, 0) scale(0.94); opacity: 0.34; }
          50% { transform: translate3d(-8%, -5%, 0) scale(1.08); opacity: 0.58; }
          100% { transform: translate3d(10%, 6%, 0) scale(1.02); opacity: 0.4; }
        }

        @keyframes bubble-ribbon-shift {
          0% { transform: translate3d(-8%, 0, 0) scaleX(0.94); opacity: 0.16; }
          50% { transform: translate3d(2%, 0, 0) scaleX(1.06); opacity: 0.3; }
          100% { transform: translate3d(12%, 0, 0) scaleX(1.18); opacity: 0.2; }
        }

        @keyframes bubble-grid-pan {
          0% { transform: translate3d(0, 0, 0) scale(1); opacity: 0.22; }
          100% { transform: translate3d(-2.5%, 1.5%, 0) scale(1.04); opacity: 0.34; }
        }
      `}
    </style>
  )
}

function AmbientBackdrop({
  theme,
  panel = false,
}: {
  theme: 'vibe-light' | 'vibe-dark'
  panel?: boolean
}) {
  const palette = motionPalette(theme)

  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 overflow-hidden rounded-[inherit]">
      <div
        className="absolute inset-[-8%]"
        style={{
          backgroundImage: `
            linear-gradient(115deg, transparent 0%, ${palette.beam} 32%, transparent 68%),
            radial-gradient(circle at 20% 18%, ${palette.mist} 0%, transparent 42%)
          `,
          filter: 'blur(28px)',
          animation: 'bubble-grid-pan 22s ease-in-out infinite alternate',
        }}
      />
      <div
        className="absolute -right-24 -top-20 h-[24rem] w-[24rem] rounded-full"
        style={{
          background: `radial-gradient(circle, ${palette.orbA} 0%, transparent 70%)`,
          filter: 'blur(46px)',
          animation: 'bubble-aurora-float 20s ease-in-out infinite alternate',
        }}
      />
      <div
        className="absolute -left-24 bottom-[-6rem] h-[26rem] w-[26rem] rounded-full"
        style={{
          background: `radial-gradient(circle, ${palette.orbB} 0%, transparent 72%)`,
          filter: 'blur(52px)',
          animation: 'bubble-aurora-drift 24s ease-in-out infinite alternate',
        }}
      />
      <div
        className="absolute inset-x-[-16%] top-[28%] h-24"
        style={{
          background: `linear-gradient(90deg, transparent 0%, ${palette.ripple} 18%, ${palette.beam} 48%, transparent 84%)`,
          filter: 'blur(28px)',
          animation: 'bubble-ribbon-shift 17s ease-in-out infinite alternate',
        }}
      />
      {panel ? (
        <div
          className="absolute inset-x-[8%] top-[38%] h-28 rounded-full"
          style={{
            background: `radial-gradient(circle at 35% 50%, ${palette.orbC} 0%, transparent 70%)`,
            filter: 'blur(34px)',
            animation: 'bubble-aurora-float 16s ease-in-out infinite alternate-reverse',
          }}
        />
      ) : null}
    </div>
  )
}

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div
      className="relative isolate min-h-screen overflow-hidden px-4 py-6 text-base-content"
      style={{
        backgroundImage: backdropImage('vibe-light'),
        backgroundSize: 'cover',
        backgroundPosition: 'center',
      }}
    >
      <StoryMotionStyles />
      <AmbientBackdrop theme="vibe-light" />
      <div className="relative z-10 mx-auto flex w-full max-w-6xl flex-col gap-6 rounded-[1.75rem] border border-base-300/70 bg-base-100/58 p-6 shadow-[0_24px_70px_rgba(15,23,42,0.12)] backdrop-blur-md">
        {children}
      </div>
    </div>
  )
}

function variantDotClass(variant: BubbleVariant) {
  switch (variant) {
    case 'info':
      return 'border-info/40 bg-info/20 text-info'
    case 'success':
      return 'border-success/40 bg-success/20 text-success'
    case 'warning':
      return 'border-warning/40 bg-warning/20 text-warning'
    case 'error':
      return 'border-error/40 bg-error/20 text-error'
    default:
      return 'border-base-300/80 bg-base-200/80 text-base-content/70'
  }
}

function InlineBubblePreview({
  variant,
  label,
  message,
}: {
  variant: BubbleVariant
  label: string
  message: string
}) {
  return (
    <div className="grid min-h-[6.25rem] grid-cols-[9rem_1fr] items-start gap-4 rounded-[1.1rem] border border-base-300/50 bg-base-100/24 px-4 py-4 backdrop-blur-sm">
      <div className="flex items-center gap-3 pt-1">
        <span className={cn('inline-flex h-3 w-3 shrink-0 rounded-full border', variantDotClass(variant))} />
        <span className="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/60">
          {label}
        </span>
      </div>
      <div className="flex items-center justify-end pr-2">
        <FloatingFieldBubble
          message={message}
          variant={variant}
          placement="label-inline"
          anchor="Anchor"
          anchorClassName={cn(
            'inline-flex h-7 items-center rounded-full border bg-base-100 px-2.5 text-[10px] font-semibold uppercase tracking-[0.14em] shadow-sm',
            variant === 'neutral'
              ? 'border-base-300/80 text-base-content/60'
              : variant === 'info'
                ? 'border-info/35 text-info'
                : variant === 'success'
                  ? 'border-success/35 text-success'
                  : variant === 'warning'
                    ? 'border-warning/35 text-warning'
                    : 'border-error/35 text-error',
          )}
        />
      </div>
    </div>
  )
}

function ThemeBubblePanel({ theme, title }: { theme: 'vibe-light' | 'vibe-dark'; title: string }) {
  return (
    <section
      data-theme={theme}
      className="relative isolate overflow-hidden rounded-[1.5rem] border border-base-300/75 p-5 text-base-content shadow-[0_18px_48px_rgba(15,23,42,0.12)]"
      style={{
        backgroundImage: backdropImage(theme),
        backgroundSize: 'cover',
        backgroundPosition: 'center',
      }}
    >
      <AmbientBackdrop theme={theme} panel />
      <div className="relative z-10 mb-4 flex items-center justify-between gap-3">
        <div>
          <p className="text-sm font-semibold">{title}</p>
          <p className="text-xs text-base-content/65">Shared surface tokens across bubble variants.</p>
        </div>
        <span className="rounded-full border border-base-300/80 bg-base-200/75 px-3 py-1 text-[11px] font-semibold uppercase tracking-[0.14em] text-base-content/65">
          {theme === 'vibe-light' ? 'Light' : 'Dark'}
        </span>
      </div>
      <div className="relative z-10 mx-auto flex w-full max-w-3xl flex-col gap-5">
        <InlineBubblePreview
          variant="neutral"
          label="Neutral"
          message="Neutral helper text keeps the same anchored bubble treatment."
        />
        <InlineBubblePreview
          variant="info"
          label="Info"
          message="Informational guidance now uses the same mature popover primitive."
        />
        <InlineBubblePreview
          variant="success"
          label="Success"
          message="Success confirms that a field value is valid and ready to submit."
        />
        <InlineBubblePreview
          variant="warning"
          label="Warning"
          message="Warning highlights a risky input before it becomes invalid."
        />
        <InlineBubblePreview
          variant="error"
          label="Error"
          message="Error clearly signals blocking validation feedback."
        />
      </div>
    </section>
  )
}

function InputCornerHarness({
  message,
  variant = 'error',
  className,
}: {
  message: string
  variant?: 'neutral' | 'info' | 'success' | 'warning' | 'error'
  className?: string
}) {
  return (
    <div className={className}>
      <label className="field">
        <span className="field-label">上游地址</span>
        <div className="relative">
          <Input readOnly value="proxy.example.com/gateway" aria-invalid={variant === 'error' ? 'true' : 'false'} />
          <FloatingFieldBubble message={message} variant={variant} />
        </div>
      </label>
    </div>
  )
}

const meta = {
  title: 'UI/FloatingFieldBubble',
  component: FloatingFieldBubble,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      page: () => <Stories title="" includePrimary={true} />,
      canvas: {
        sourceState: 'hidden',
      },
      controls: {
        hideNoControlsWarning: true,
      },
    },
  },
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
  argTypes: {
    className: {
      control: false,
    },
  },
} satisfies Meta<typeof FloatingFieldBubble>

export default meta

type Story = StoryObj<typeof meta>

export const StateGallery: Story = {
  args: {
    message: 'Shared bubble gallery',
    variant: 'neutral',
    placement: 'label-inline',
  },
  render: () => (
    <div className="grid gap-6">
      <ThemeBubblePanel theme="vibe-light" title="Shared Bubbles" />
      <ThemeBubblePanel theme="vibe-dark" title="Shared Bubbles" />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const bubbles = canvasElement.ownerDocument.body.querySelectorAll('[role="status"], [role="alert"]')

    await expect(bubbles.length).toBeGreaterThan(0)
    bubbles.forEach((bubble) => {
      expectBubbleArrowInsideFlatSegment(bubble)
    })
  },
}

export const InputCornerError: Story = {
  args: {
    message: '请填写 http(s) 的绝对 URL，例如 https://proxy.example.com/gateway',
    variant: 'error',
    placement: 'input-corner',
  },
  render: (args) => <InputCornerHarness message={args.message} variant={args.variant} className="max-w-xl" />,
  play: async ({ canvasElement }) => {
    const doc = within(canvasElement.ownerDocument.body)
    const bubble = await doc.findByRole('alert')

    expectBubbleArrowInsideFlatSegment(bubble)
  },
}

export const OverflowAncestor: Story = {
  args: {
    message: '气泡内容会通过 portal 渲染到 document.body，不会被祖先 overflow-hidden 裁掉。',
    variant: 'warning',
    placement: 'input-corner',
  },
  render: (args) => (
    <div className="grid gap-4">
      <p className="text-sm text-base-content/70">The yellow card intentionally uses `overflow-hidden` and a narrow width.</p>
      <div className="overflow-hidden rounded-[1.6rem] border border-warning/35 bg-warning/10 p-4 shadow-sm">
        <InputCornerHarness
          message={args.message}
          variant={args.variant}
          className="max-w-[15rem]"
        />
      </div>
    </div>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const doc = within(canvasElement.ownerDocument.body)
    await expect(canvas.queryByRole('status')).toBeNull()
    await expect(doc.getByRole('status')).toBeInTheDocument()
  },
}

export const ViewportEdgeAware: Story = {
  args: {
    message: '在视口右下角时，气泡会自动回避边缘并保持完整可见。',
    variant: 'info',
    placement: 'input-corner',
  },
  render: (args) => (
    <div className="flex min-h-[70vh] items-end justify-end rounded-[1.6rem] border border-base-300/70 bg-base-200/55 p-4">
      <InputCornerHarness
        message={args.message}
        variant={args.variant}
        className="w-[14rem]"
      />
    </div>
  ),
  play: async ({ canvasElement }) => {
    const doc = within(canvasElement.ownerDocument.body)
    const bubble = await doc.findByRole('status')
    const rect = bubble.getBoundingClientRect()
    await expect(rect.right <= window.innerWidth - 1).toBe(true)
    await expect(rect.bottom <= window.innerHeight - 1).toBe(true)
    await expect((bubble.getAttribute('data-side') ?? '').length > 0).toBe(true)
    expectBubbleArrowInsideFlatSegment(bubble)
  },
}
