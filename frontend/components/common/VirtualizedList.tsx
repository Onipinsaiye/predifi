"use client";

import { useRef, useState, useCallback, useMemo, type ReactNode } from "react";

/**
 * VirtualizedList — windows a long, uniform-height list to a fixed-height
 * viewport so only the rows near the visible area are ever mounted.
 *
 * Below `threshold` items it renders everything directly (no windowing
 * overhead, no scroll container) since short lists don't need it. Above the
 * threshold it keeps a fixed-height scroll viewport and mounts only the
 * visible rows plus `overscan` rows on each side, positioning them with
 * `transform` inside a spacer sized to the full list height so native
 * scrollbars and scroll-by-keyboard (Space/PageDown/Home/End/arrows) keep
 * working exactly as they would for a non-virtualized list.
 *
 * Row identity is keyed by `itemKey`, not by array index, so reordering or
 * insertion never causes rows to swap content under an unrelated key.
 */
export interface VirtualizedListProps<T> {
  items: readonly T[];
  /** Stable identity for a row — used as the React key. */
  itemKey: (item: T, index: number) => string | number;
  /** Fixed row height in pixels. All rows must share this height. */
  rowHeight: number;
  renderRow: (item: T, index: number) => ReactNode;
  /** Height of the scroll viewport once virtualization kicks in. */
  height?: number;
  /** Item count above which rows are windowed instead of rendered directly. */
  threshold?: number;
  /** Extra rows kept mounted above/below the visible range. */
  overscan?: number;
  ariaLabel?: string;
  className?: string;
}

const DEFAULT_THRESHOLD = 50;
const DEFAULT_OVERSCAN = 6;

export function VirtualizedList<T>({
  items,
  itemKey,
  rowHeight,
  renderRow,
  height = 480,
  threshold = DEFAULT_THRESHOLD,
  overscan = DEFAULT_OVERSCAN,
  ariaLabel,
  className,
}: VirtualizedListProps<T>) {
  const [scrollTop, setScrollTop] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const handleScroll = useCallback(() => {
    const node = containerRef.current;
    if (node) setScrollTop(node.scrollTop);
  }, []);

  if (items.length <= threshold) {
    return (
      <div role="list" aria-label={ariaLabel} className={className}>
        {items.map((item, index) => (
          <div role="listitem" key={itemKey(item, index)}>
            {renderRow(item, index)}
          </div>
        ))}
      </div>
    );
  }

  const totalHeight = items.length * rowHeight;
  const visibleCount = Math.ceil(height / rowHeight);
  const startIndex = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const endIndex = Math.min(items.length, startIndex + visibleCount + overscan * 2);

  const visibleRows = useMemo(() => {
    const rows: ReactNode[] = [];
    for (let index = startIndex; index < endIndex; index += 1) {
      const item = items[index];
      rows.push(
        <div
          role="listitem"
          key={itemKey(item, index)}
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            right: 0,
            height: rowHeight,
            transform: `translateY(${index * rowHeight}px)`,
          }}
        >
          {renderRow(item, index)}
        </div>,
      );
    }
    return rows;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, startIndex, endIndex, rowHeight, itemKey, renderRow]);

  return (
    <div
      ref={containerRef}
      role="list"
      aria-label={ariaLabel}
      tabIndex={0}
      onScroll={handleScroll}
      className={className}
      style={{ height, overflowY: "auto", position: "relative" }}
    >
      <div style={{ height: totalHeight, position: "relative" }}>{visibleRows}</div>
    </div>
  );
}
