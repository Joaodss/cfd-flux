// shadcn/ui's Popover, on Radix primitives.
import { Popover as P } from 'radix-ui'
import type { ComponentProps } from 'react'
import { cn } from '../../lib/utils.ts'

export const Popover = P.Root
export const PopoverTrigger = P.Trigger

export function PopoverContent({
  className,
  align = 'end',
  sideOffset = 4,
  ...props
}: ComponentProps<typeof P.Content>) {
  return (
    <P.Portal>
      <P.Content
        align={align}
        sideOffset={sideOffset}
        className={cn(
          'z-50 w-[28rem] max-w-[calc(100vw-2rem)] rounded-md border border-border bg-popover p-2 text-popover-foreground shadow-md outline-none',
          className,
        )}
        {...props}
      />
    </P.Portal>
  )
}
