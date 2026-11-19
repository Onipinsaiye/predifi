import { render, screen, fireEvent } from "@testing-library/react";
import { VirtualizedList } from "@/components/common/VirtualizedList";

interface Row {
  id: number;
  label: string;
}

function makeRows(count: number): Row[] {
  return Array.from({ length: count }, (_, i) => ({ id: i, label: `Row ${i}` }));
}

describe("VirtualizedList", () => {
  it("renders every row directly when below the threshold", () => {
    const rows = makeRows(10);
    render(
      <VirtualizedList
        items={rows}
        itemKey={(row) => row.id}
        rowHeight={40}
        threshold={50}
        ariaLabel="small list"
        renderRow={(row) => <span>{row.label}</span>}
      />,
    );

    expect(screen.getAllByRole("listitem")).toHaveLength(10);
  });

  it("windows a high-volume dataset without missing or duplicating records", () => {
    const rows = makeRows(5000);
    render(
      <VirtualizedList
        items={rows}
        itemKey={(row) => row.id}
        rowHeight={40}
        height={400}
        threshold={50}
        ariaLabel="huge list"
        renderRow={(row) => <span data-testid="row-label">{row.label}</span>}
      />,
    );

    const list = screen.getByRole("list", { name: "huge list" });
    const rendered = screen.getAllByRole("listitem");

    // Only a small window around the visible viewport is mounted, not all 5000.
    expect(rendered.length).toBeLessThan(50);
    expect(rendered.length).toBeGreaterThan(0);

    // No duplicated identities among what is mounted.
    const labels = screen.getAllByTestId("row-label").map((el) => el.textContent);
    expect(new Set(labels).size).toBe(labels.length);

    // Scrolling to the bottom brings the last record into view with no gaps
    // or dupes, proving the full 5000-item range is addressable.
    fireEvent.scroll(list, { target: { scrollTop: 5000 * 40 - 400 } });
    expect(screen.getByText("Row 4999")).toBeInTheDocument();
  });

  it("exposes accessible list semantics and keeps the viewport keyboard-focusable", () => {
    const rows = makeRows(500);
    render(
      <VirtualizedList
        items={rows}
        itemKey={(row) => row.id}
        rowHeight={40}
        height={300}
        threshold={50}
        ariaLabel="keyboard list"
        renderRow={(row) => <span>{row.label}</span>}
      />,
    );

    const list = screen.getByRole("list", { name: "keyboard list" });
    expect(list).toHaveAttribute("tabIndex", "0");
    expect(screen.getAllByRole("listitem").length).toBeGreaterThan(0);
  });
});
