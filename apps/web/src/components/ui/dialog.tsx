// shadcn/ui's Dialog (the parts the editor uses), on Radix primitives: a modal with focus
// trapping, Escape to close and an overlay.
import { X } from 'lucide-react'
import { Dialog as D } from 'radix-ui'
import type { ComponentProps, ReactNode } from 'react'
import { cn } from '../../lib/utils.ts'

export const Dialog = D.Root

export function DialogContent({
  className,
  title,
  description,
  children,
  ...props
}: ComponentProps<typeof D.Content> & {
  title: string
  description?: string
  children: ReactNode
}) {
  return (
    <D.Portal>
      <D.Overlay className="fixed inset-0 z-50 bg-black/50" />
      <D.Content
        className={cn(
          'fixed top-1/2 left-1/2 z-50 flex max-h-[85vh] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-lg border border-border bg-popover p-4 text-popover-foreground shadow-lg outline-none',
          className,
        )}
        {...props}
      >
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <D.Title className="font-semibold">{title}</D.Title>
            {description ? (
              <D.Description className="text-xs text-muted-foreground">{description}</D.Description>
            ) : (
              <D.Description className="sr-only">{title}</D.Description>
            )}
          </div>
          <D.Close
            aria-label="Close"
            className="grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-accent [&_svg]:size-4"
          >
            <X />
          </D.Close>
        </div>
        {children}
      </D.Content>
    </D.Portal>
  )
}
