"use client";

import React from "react";
import { Share2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import SocialIcon, { SocialIconId } from "@/components/ui/SocialIcon";
import { useCopyToClipboard } from "@/lib/hooks/useCopyToClipboard";

export interface ShareButtonProps {
  /** The URL to share */
  url: string;
  /** The text/title to share */
  title?: string;
  /** Additional description text (for some social networks) */
  text?: string;
  /** Custom class names */
  className?: string;
  /** Which social networks to include (defaults to all) */
  networks?: SocialIconId[];
}

const defaultNetworks: SocialIconId[] = ["x", "telegram", "reddit", "discord"];

function getShareUrl(network: SocialIconId, url: string, title?: string) {
  const encodedUrl = encodeURIComponent(url);
  const encodedTitle = title ? encodeURIComponent(title) : "";

  switch (network) {
    case "x":
      return `https://twitter.com/intent/tweet?url=${encodedUrl}${encodedTitle ? `&text=${encodedTitle}` : ""}`;
    case "telegram":
      return `https://t.me/share/url?url=${encodedUrl}${encodedTitle ? `&text=${encodedTitle}` : ""}`;
    case "reddit":
      return `https://reddit.com/submit?url=${encodedUrl}${encodedTitle ? `&title=${encodedTitle}` : ""}`;
    case "discord":
      // Discord doesn't have a direct share link, so just copy the URL
      return null;
    default:
      return null;
  }
}

export function ShareButton({
  url,
  title,
  text,
  className,
  networks = defaultNetworks,
}: ShareButtonProps) {
  const [isOpen, setIsOpen] = React.useState(false);
  const buttonRef = React.useRef<HTMLButtonElement>(null);
  const dropdownRef = React.useRef<HTMLDivElement>(null);
  const itemRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const { copy } = useCopyToClipboard({
    successTitle: "Link copied!",
    successDescription: "The link has been copied to your clipboard.",
  });

  const closeAndRestoreFocus = React.useCallback(() => {
    setIsOpen(false);
    buttonRef.current?.focus();
  }, []);

  const focusItem = React.useCallback(
    (index: number) => {
      const count = networks.length;
      if (count === 0) return;
      const next = ((index % count) + count) % count;
      itemRefs.current[next]?.focus();
    },
    [networks.length],
  );

  const handleTriggerKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setIsOpen(true);
      // Focus first/last item after render
      requestAnimationFrame(() => {
        focusItem(event.key === "ArrowDown" ? 0 : networks.length - 1);
      });
    } else if (event.key === "Escape" && isOpen) {
      event.preventDefault();
      closeAndRestoreFocus();
    }
  };

  const handleItemKeyDown = (
    event: React.KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) => {
    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        event.preventDefault();
        focusItem(index + 1);
        break;
      case "ArrowLeft":
      case "ArrowUp":
        event.preventDefault();
        focusItem(index - 1);
        break;
      case "Home":
        event.preventDefault();
        focusItem(0);
        break;
      case "End":
        event.preventDefault();
        focusItem(networks.length - 1);
        break;
      case "Escape":
        event.preventDefault();
        closeAndRestoreFocus();
        break;
      case "Tab":
        // Allow natural tabbing out, but close the menu
        setIsOpen(false);
        break;
      default:
        break;
    }
  };

  const handleShare = async () => {
    if (navigator.share) {
      try {
        await navigator.share({
          title,
          text,
          url,
        });
      } catch {
        // User canceled or share failed, open custom dropdown
        setIsOpen(!isOpen);
      }
    } else {
      setIsOpen(!isOpen);
    }
  };

  const handleNetworkClick = (network: SocialIconId) => {
    const shareUrl = getShareUrl(network, url, title);
    if (shareUrl) {
      window.open(shareUrl, "_blank", "noopener,noreferrer");
    } else if (network === "discord") {
      // For Discord, copy the URL and show a toast
      void copy(url);
    }
    closeAndRestoreFocus();
  };

  // Close dropdown when clicking outside or pressing Escape anywhere
  React.useEffect(() => {
    if (!isOpen) return;

    const handleClickOutside = (event: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(event.target as Node) &&
        buttonRef.current &&
        !buttonRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        closeAndRestoreFocus();
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen, closeAndRestoreFocus]);

  // Move focus into the menu when it opens via keyboard/mouse
  React.useEffect(() => {
    if (isOpen) {
      requestAnimationFrame(() => focusItem(0));
    }
  }, [isOpen, focusItem]);

  return (
    <div className="relative">
      <Button
        ref={buttonRef}
        onClick={handleShare}
        onKeyDown={handleTriggerKeyDown}
        variant="tertiary"
        size="icon"
        className={cn(className)}
        aria-haspopup="menu"
        aria-expanded={isOpen}
        aria-controls={isOpen ? "share-menu" : undefined}
      >
        <Share2 className="w-4 h-4" />
        <span className="sr-only">Share</span>
      </Button>

      {isOpen && (
        <div
          ref={dropdownRef}
          id="share-menu"
          role="menu"
          aria-label="Share options"
          className="absolute right-0 mt-2 z-50 rounded-lg border border-input bg-background p-2 shadow-lg"
        >
          <div className="flex items-center gap-2">
            {networks.map((network, index) => (
              <button
                key={network}
                ref={(el) => {
                  itemRefs.current[index] = el;
                }}
                type="button"
                role="menuitem"
                onClick={() => handleNetworkClick(network)}
                onKeyDown={(event) => handleItemKeyDown(event, index)}
                className="min-w-[44px] min-h-[44px] inline-flex items-center justify-center rounded-md hover:bg-accent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                aria-label={`Share on ${network}`}
              >
                <SocialIcon id={network} label={network} className="w-5 h-5" />
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
