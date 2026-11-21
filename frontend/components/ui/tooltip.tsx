"use client";

import * as React from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { cn } from "@/lib/utils";

const TooltipProvider = TooltipPrimitive.Provider;

const TooltipRoot = TooltipPrimitive.Root;

const TooltipTrigger = TooltipPrimitive.Trigger;

const TooltipContent = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 4, ...props }, ref) => (
  <TooltipPrimitive.Content
    ref={ref}
    sideOffset={sideOffset}
    className={cn(
      "z-50 overflow-hidden rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground animate-in fade-in-0 zoom-in-95 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2",
      className
    )}
    {...props}
  />
));
TooltipContent.displayName = TooltipPrimitive.Content.displayName;

const TooltipArrow = React.forwardRef<
  React.ElementRef<typeof TooltipPrimitive.Arrow>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Arrow>
>(({ className, ...props }, ref) => (
  <TooltipPrimitive.Arrow
    ref={ref}
    className={cn("fill-primary", className)}
    {...props}
  />
));
TooltipArrow.displayName = TooltipPrimitive.Arrow.displayName;

export interface TooltipProps {
  content: React.ReactNode;
  children: React.ReactNode;
  side?: "top" | "right" | "bottom" | "left";
  delayDuration?: number;
  showArrow?: boolean;
  /**
   * When true, the tooltip content is also exposed as an accessible
   * description of the trigger via `aria-describedby`, so screen readers
   * announce it on focus (not just on hover). Keyboard focus opens the
   * tooltip, and the content remains reachable without a pointer.
   */
  asDescription?: boolean;
  /** Optional id for the description element (auto-generated if omitted). */
  descriptionId?: string;
}

const Tooltip = React.forwardRef<HTMLButtonElement, TooltipProps>(
  (
    {
      content,
      children,
      side = "top",
      delayDuration = 200,
      showArrow = true,
      asDescription = false,
      descriptionId,
    },
    ref
  ) => {
    const generatedId = React.useId();
    const contentId = descriptionId ?? `tooltip-${generatedId}`;

    const trigger = asDescription ? (
      <TooltipTrigger ref={ref} asChild aria-describedby={contentId}>
        {children}
      </TooltipTrigger>
    ) : (
      <TooltipTrigger ref={ref} asChild>
        {children}
      </TooltipTrigger>
    );

    return (
      <TooltipProvider delayDuration={delayDuration}>
        <TooltipRoot>
          {trigger}
          <TooltipContent side={side} id={asDescription ? contentId : undefined}>
            {content}
            {showArrow && <TooltipArrow />}
          </TooltipContent>
        </TooltipRoot>
      </TooltipProvider>
    );
  }
);

Tooltip.displayName = "Tooltip";

export {
  Tooltip,
  TooltipProvider,
  TooltipRoot,
  TooltipTrigger,
  TooltipContent,
  TooltipArrow,
};
