import { beforeEach, expect, it, vi } from "vitest";

// Each query's SQL, answered by `answer`.
const sqls: string[] = [];
let answer: (sql: string) => { columns: string[]; rows: unknown[][] } = () => ({ columns: [], rows: [] });
vi.mock("../duck", () => ({
  rowsView: (t: string) => `${t}__rn`,
  runSql: vi.fn(async (sql: string) => {
    sqls.push(sql);
    const r = answer(sql);
    return { ...r, rowCount: r.rows.length, truncated: false };
  }),
}));
import { psmPeaks, psmRecord } from "./spectrum";
import type { TableRef } from "../report";

const P = {
  table: "x_psm",
  cols: new Set(["peptidoform", "run_file_name", "scan", "charge", "mz_array", "intensity_array"]),
  types: { scan: "INTEGER[]" },
} as unknown as TableRef;
const key = { peptidoform: "PEPTIDE", run_file_name: "r1", scan: 7, charge: 2 };
const peaksRow = (pf: string) => ({ columns: ["mz_array", "intensity_array", "charge", "peptidoform"], rows: [[[1, 2], [3, 4], 2, pf]] });

beforeEach(() => {
  sqls.length = 0;
});

it("reads a PSM with a known row number directly", async () => {
  answer = () => peaksRow("PEPTIDE");
  const r = await psmRecord(P, { ...key, row: 120000 });
  expect(r?.mz_array).toEqual([1, 2]);
  expect(sqls).toHaveLength(1);
  expect(sqls[0]).toContain('FROM "x_psm__rn" WHERE file_row_number = 120000');
});

it("finds a PSM without a row number by its narrow columns only", async () => {
  answer = (sql) => (sql.startsWith("SELECT file_row_number") ? { columns: ["file_row_number"], rows: [[55]] } : peaksRow("PEPTIDE"));
  await psmRecord(P, key);
  expect(sqls).toHaveLength(2);
  // The search selects no peak arrays: a filtered query that does makes
  // duckdb-wasm download them for every row group it passes.
  expect(sqls[0]).toMatch(/^SELECT file_row_number FROM "x_psm__rn" WHERE peptidoform = 'PEPTIDE'/);
  expect(sqls[0]).not.toContain("mz_array");
  expect(sqls[1]).toContain("WHERE file_row_number = 55");
});

it("searches again when the row number points at another PSM", async () => {
  answer = (sql) =>
    sql.startsWith("SELECT file_row_number") ? { columns: ["file_row_number"], rows: [[9]] } : sql.includes("= 9") ? peaksRow("PEPTIDE") : peaksRow("OTHER");
  const r = await psmPeaks(P, "PEPTIDE", { run_file_name: "r1", scan: [7], charge: 2, file_row_number: 3 });
  expect(r?.peptidoform).toBe("PEPTIDE");
  expect(sqls.map((s) => s.slice(0, 22))).toEqual(["SELECT \"mz_array\", \"in", "SELECT file_row_number", "SELECT \"mz_array\", \"in"]);
});
