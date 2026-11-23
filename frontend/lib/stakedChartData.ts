/** Plotted values for the dashboard "Total staked" chart (USD per month). */
export interface StakedPoint {
  name: string;
  value: number;
  active?: boolean;
}

export const STAKED_CHART_DATA: StakedPoint[] = [
  { name: "JAN", value: 35000 },
  { name: "FEB", value: 45000 },
  { name: "MAR", value: 35000 },
  { name: "APR", value: 30000 },
  { name: "MAY", value: 65000, active: true },
  { name: "JUN", value: 15000 },
  { name: "JUL", value: 50000 },
  { name: "AUG", value: 25000 },
  { name: "SEP", value: 45000 },
  { name: "OCT", value: 75000 },
  { name: "NOV", value: 45000 },
  { name: "DEC", value: 60000 },
];
