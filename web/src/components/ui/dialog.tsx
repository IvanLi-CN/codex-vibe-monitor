import * as DialogPrimitive from "@radix-ui/react-dialog";
import * as React from "react";
import { AppIcon } from "../../features/shared/AppIcon";
import { cn } from "../../lib/utils";
import { OverlayHostProvider } from "./overlay-host";
import { useOverlayHostElement, useResolvedOverlayContainer } from "./use-overlay-host";

const Dialog = DialogPrimitive.Root;

const DialogTrigger = DialogPrimitive.Trigger;

function DialogPortal({
  container,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogPrimitive.Portal> & {
  container?: HTMLElement | null;
}) {
  const resolvedContainer = useResolvedOverlayContainer(container);
  return <DialogPrimitive.Portal container={resolvedContainer ?? undefined} {...props} />;
}

const DialogClose = DialogPrimitive.Close;

type DialogMobileLayout = "sheet" | "centered";

const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn(
      "dialog-overlay fixed inset-0 z-[80]",
      "data-[state=open]:animate-in data-[state=closed]:animate-out",
      "data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0",
      className,
    )}
    {...props}
  />
));
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName;

const DialogContent = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content> & {
    container?: HTMLElement | null;
    mobileLayout?: DialogMobileLayout;
  }
>(({ className, children, container, mobileLayout = "sheet", ...props }, ref) => {
  const resolvedContainer = useResolvedOverlayContainer(container);
  const { hostElement, ref: hostRef } = useOverlayHostElement<HTMLDivElement>(undefined);
  const hostValue = hostElement ?? (container === undefined ? resolvedContainer : container);
  const mobileLayoutClassName =
    mobileLayout === "centered"
      ? "left-1/2 right-auto bottom-auto top-1/2 max-h-[calc(100dvh-1rem)] w-[calc(100vw-1rem)] max-w-[34rem] -translate-x-1/2 -translate-y-1/2 rounded-[1.75rem] border px-0 pb-0 pt-0"
      : "inset-x-0 bottom-0 top-auto max-h-[min(100dvh-0.5rem,100dvh)] w-full max-w-full translate-x-0 translate-y-0 rounded-t-[1.75rem] rounded-b-none border border-x-0 border-b-0 px-0 pb-[env(safe-area-inset-bottom)] pt-0";
  const contentRef = React.useCallback(
    (node: React.ElementRef<typeof DialogPrimitive.Content> | null) => {
      if (typeof ref === "function") {
        ref(node);
      } else if (ref) {
        ref.current = node;
      }
    },
    [ref],
  );

  return (
    <DialogPortal container={resolvedContainer}>
      <DialogOverlay />
      <div ref={hostRef} className="fixed inset-0 z-[81] pointer-events-none">
        <OverlayHostProvider value={hostValue}>
          <DialogPrimitive.Content
            ref={contentRef}
            data-mobile-layout={mobileLayout}
            className={cn(
              "dialog-surface pointer-events-auto fixed outline-none",
              mobileLayoutClassName,
              "data-[state=open]:animate-in data-[state=closed]:animate-out",
              "data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0",
              "data-[state=closed]:slide-out-to-bottom-6 data-[state=open]:slide-in-from-bottom-6",
              "desktop:left-1/2 desktop:right-auto desktop:bottom-auto desktop:top-1/2 desktop:max-h-[calc(100dvh-2rem)] desktop:w-[min(34rem,calc(100vw-2rem))] desktop:-translate-x-1/2 desktop:-translate-y-1/2 desktop:rounded-[1.75rem] desktop:border desktop:px-0 desktop:pb-0",
              "desktop:data-[state=closed]:slide-out-to-bottom-0 desktop:data-[state=open]:slide-in-from-bottom-0 desktop:data-[state=closed]:zoom-out-95 desktop:data-[state=open]:zoom-in-95",
              className,
            )}
            {...props}
          >
            {children}
          </DialogPrimitive.Content>
        </OverlayHostProvider>
      </div>
    </DialogPortal>
  );
});
DialogContent.displayName = DialogPrimitive.Content.displayName;

function DialogHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("flex flex-col gap-1.5", className)} {...props} />;
}

function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("flex flex-col-reverse gap-3 desktop:flex-row desktop:justify-end", className)}
      {...props}
    />
  );
}

const DialogTitle = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn("text-xl font-semibold tracking-tight text-base-content", className)}
    {...props}
  />
));
DialogTitle.displayName = DialogPrimitive.Title.displayName;

const DialogDescription = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    ref={ref}
    className={cn("text-sm leading-7 text-base-content/82", className)}
    {...props}
  />
));
DialogDescription.displayName = DialogPrimitive.Description.displayName;

function DialogCloseIcon({
  className,
  ...props
}: React.ComponentPropsWithoutRef<typeof DialogClose>) {
  return (
    <DialogClose
      className={cn(
        "inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-full text-base-content/78 transition-colors",
        "hover:bg-base-200 hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
        "disabled:pointer-events-none disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <AppIcon name="close" className="h-5 w-5" aria-hidden />
      <span className="sr-only">Close</span>
    </DialogClose>
  );
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
};
