import { expect, it } from "vitest";
import { OVERVIEW_CHARTS, countsSql } from "./overview";

it("counts several columns in one GROUPING SETS scan", () => {
  const sql = countsSql("ev", "NOT is_decoy", ["run_file_name", "charge"]);
  expect(sql).toContain('FROM "ev" WHERE NOT is_decoy');
  expect(sql).toContain("GROUP BY GROUPING SETS ((run_file_name), (charge))");
  expect(sql).toContain("WHEN grouping(charge) = 0 THEN 'charge'");
  expect(sql).toContain("WHEN grouping(charge) = 0 THEN charge::VARCHAR");
});

it("computes the shared counts first and whole-sequence charts last", () => {
  expect(OVERVIEW_CHARTS.slice(0, 3)).toEqual(["charge", "mc", "perRun"]);
  expect(OVERVIEW_CHARTS.slice(-2)).toEqual(["length", "ppg"]);
  expect(new Set(OVERVIEW_CHARTS).size).toBe(10);
});
