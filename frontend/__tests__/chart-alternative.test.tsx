/**
 * Non-visual chart alternatives (#77): the summary and data table must carry
 * the same values, units and range as the plotted chart fixtures.
 */

import { render, screen, within } from "@testing-library/react";
import { axe } from "jest-axe";
import { ChartAlternative } from "@/components/ui/chart-alternative";
import { buildChartAlternative } from "@/lib/chartAccessibility";
import { STAKED_CHART_DATA } from "@/lib/stakedChartData";
import { formatChartValue } from "@/lib/stakeFilters";

const stakedData = STAKED_CHART_DATA.map((d) => ({ label: d.name, value: d.value }));

describe("buildChartAlternative", () => {
  it("emits one row per plotted point with identical values", () => {
    const { rows } = buildChartAlternative({
      title: "Total staked",
      unit: "USD",
      data: stakedData,
    });
    expect(rows).toHaveLength(STAKED_CHART_DATA.length);
    rows.forEach((row, i) => {
      expect(row.label).toBe(STAKED_CHART_DATA[i].name);
      expect(row.value).toBe(STAKED_CHART_DATA[i].value);
      expect(row.formatted).toBe(formatChartValue(STAKED_CHART_DATA[i].value));
    });
  });

  it("summary states title, unit, range, extremes and total", () => {
    const { summary } = buildChartAlternative({
      title: "Total staked",
      unit: "USD",
      rangeLabel: "January to December",
      data: stakedData,
    });
    expect(summary).toContain("Total staked");
    expect(summary).toContain("January to December");
    expect(summary).toContain("in USD");
    expect(summary).toContain("12 data points");
    expect(summary).toContain("Highest OCT at $75.0K");
    expect(summary).toContain("lowest JUN at $15.0K");
    expect(summary).toContain("total $525.0K");
  });

  it("includes shares for distribution charts", () => {
    const { summary, rows } = buildChartAlternative({
      title: "Win / Loss",
      unit: "predictions",
      showShare: true,
      formatValue: String,
      data: [
        { label: "Won", value: 6 },
        { label: "Lost", value: 3 },
        { label: "Pending", value: 1 },
      ],
    });
    expect(rows.map((r) => r.share)).toEqual(["60%", "30%", "10%"]);
    expect(summary).toContain("Won 6 (60%)");
    expect(summary).toContain("Total 10.");
  });

  it("handles empty data without throwing", () => {
    const { summary, rows } = buildChartAlternative({
      title: "Total staked",
      unit: "USD",
      data: [],
    });
    expect(rows).toEqual([]);
    expect(summary).toContain("no data available");
  });
});

describe("<ChartAlternative />", () => {
  function renderStaked() {
    return render(
      <ChartAlternative
        title="Total staked"
        unit="USD"
        rangeLabel="January to December"
        data={stakedData}
      />,
    );
  }

  it("renders a table with a caption, headers with units, and every value", () => {
    renderStaked();
    const table = screen.getByRole("table", { hidden: true });
    expect(within(table).getByText("Total staked, January to December")).toBeInTheDocument();
    expect(
      within(table).getByRole("columnheader", { name: "Total staked (USD)", hidden: true }),
    ).toBeInTheDocument();

    const bodyRows = within(table).getAllByRole("row", { hidden: true }).slice(1);
    expect(bodyRows).toHaveLength(12);
    STAKED_CHART_DATA.forEach((d, i) => {
      expect(within(bodyRows[i]).getByRole("rowheader", { hidden: true })).toHaveTextContent(d.name);
      expect(within(bodyRows[i]).getByRole("cell", { hidden: true })).toHaveTextContent(
        formatChartValue(d.value),
      );
    });
  });

  it("exposes the summary once and links it to the table", () => {
    const { container } = renderStaked();
    const summaries = container.querySelectorAll("p.sr-only");
    expect(summaries).toHaveLength(1);
    const table = container.querySelector("table")!;
    expect(table.getAttribute("aria-describedby")).toBe(summaries[0].id);
  });

  it("has no axe violations", async () => {
    const { container } = renderStaked();
    expect(await axe(container)).toHaveNoViolations();
  });
});
