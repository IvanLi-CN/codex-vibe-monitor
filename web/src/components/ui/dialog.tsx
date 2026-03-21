import * as React from 'react'
import * as DialogPrimitive from '@radix-ui/react-dialog'
import { AppIcon } from '../AppIcon'
import { cn } from '../../lib/utils'
import {
  useOverlayHostElement,
  useResolvedOverlayContainer,
} from './use-overlay-host'
import { OverlayHostProvider } from './overlay-host'

const Dialog = DialogPrimitive.Root

const DialogTrigger = DialogPrimitive.Trigger

function DialogPortal({
  container,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Portal> & {
  container?: HTMLElement | null
}) {
  const resolvedContainer = useResolvedOverlayContainer(container)
  return <DialogPrimitive.Portal container={resolvedContainer ?? undefined} {...props} />
}

const DialogClose = DialogPrimitive.Close

const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn(
      'dialog-overlay fixed inset-0 z-[80]',
      'data-[state=open]:animate-in data-[state=closed]:animate-out',
      'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
      className,
    )}
    {...props}
  />
))
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName

const DialogContent = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> & {
    container?: HTMLElement | null
  }
>(({ className, children, container, ...props }, ref) => {
  const resolvedContainer = useResolvedOverlayContainer(container)
  const { hostElement, ref: contentRef } = useOverlayHostElement(ref, resolvedContainer)

  return (
    <DialogPortal container={resolvedContainer}>
      <DialogOverlay />
      <OverlayHostProvider value={hostElement}>
        <DialogPrimitive.Content
          ref={contentRef}
          className={cn(
            'dialog-surface fixed left-1/2 top-1/2 z-[81] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2',
            'rounded-[1.75rem] border outline-none',
            'data-[state=open]:animate-in data-[state=closed]:animate-out',
            'data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0',
            'data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95',
            className,
          )}
          {...props}
        >
          {children}
        </DialogPrimitive.Content>
      </OverlayHostProvider>
    </DialogPortal>
  )
})
DialogContent.displayName = DialogPrimitive.Content.displayName

function DialogHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('flex flex-col gap-1.5', className)} {...props} />
}

function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('flex justify-end gap-3', className)} {...props} />
}

const DialogTitle = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn('text-xl font-semibold tracking-tight text-base-content', className)}
    {...props}
  />
))
DialogTitle.displayName = DialogPrimitive.Title.displayName

const DialogDescription = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn('text-sm leading-7 text-base-content/82', className)}
    {...props}
  />
))
DialogDescription.displayName = DialogPrimitive.Description.displayName

function DialogCloseIcon({ className, ...props }: React.ComponentPropsWithoutRef<typeof DialogClose>) {
  return (
    <DialogClose
      className={cn(
        'inline-flex h-10 w-10 items-center justify-center rounded-full text-base-content/78 transition-colors',
        'hover:bg-base-200 hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
        'disabled:pointer-events-none disabled:opacity-50',
        className,
      )}
      {...props}
    >
      <AppIcon name="close" className="h-5 w-5" aria-hidden />
      <span className="sr-only">Close</span>
    </DialogClose>
  )
}

export {
  Dialog,
  DialogClose,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
}
