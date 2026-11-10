// The spectrum viewer: one PSM's identification and peaks from the psm view,
// the other PSMs of its peptidoform, and one of those PSMs' peaks.
import { rowsView, runSql } from "../duck";
import { ident, lit, objects, type TableRef } from "../report";
import { COL } from "../qpx";

/** A PSM of the psm view, as the tables that open the viewer know it. */
export interface PsmKey {
  peptidoform: string;
  run_file_name?: string | null;
  scan?: number | null;
  charge?: number | null;
  /** The PSM's `file_row_number` in the psm view's file, when the list it
   * was picked from knows it: its peaks are then read without a search. */
  row?: number | null;
}

/** Columns of the PSM row the viewer reads (those the view has). */
const SPECTRUM_COLUMNS = [COL.peptidoform, COL.charge, COL.run_file_name, COL.scan, COL.calculated_mz, COL.observed_mz, COL.posterior_error_probability,
  COL.rt, COL.cv_params, COL.mz_array, COL.intensity_array, COL.charge_array, COL.ion_type_array];

// A result row's value of a column, matched case-insensitively.
function rowValue(columns: string[], row: unknown[] | undefined, col: string): unknown {
  const j = columns.findIndex((c) => c.toLowerCase() === col);
  return j >= 0 ? row?.[j] : undefined;
}

/** Why a psm-view result row cannot open the spectrum viewer, or "" when it
 * can; `peaksRead` when the row's peak arrays were read. */
export function whyNoSpectrum(columns: string[], row: unknown[] | undefined, peaksRead: boolean): string {
  const pf = rowValue(columns, row, COL.peptidoform);
  if (typeof pf !== "string" || !pf) return "This PSM has no peptidoform";
  if (peaksRead) {
    const mz = rowValue(columns, row, COL.mz_array);
    if (!Array.isArray(mz) || !mz.length) return "This PSM has no stored peaks";
  }
  return "";
}

/** The PsmKey of a psm-view result row. */
export function psmKeyOf(columns: string[], row: unknown[] | undefined): PsmKey {
  const scan = rowValue(columns, row, COL.scan);
  const charge = rowValue(columns, row, COL.charge);
  const run = rowValue(columns, row, COL.run_file_name);
  return {
    peptidoform: String(rowValue(columns, row, COL.peptidoform)),
    run_file_name: run == null ? null : String(run),
    scan: Array.isArray(scan) ? (scan[0] as number) ?? null : ((scan as number) ?? null),
    charge: (charge as number) ?? null,
  };
}

// scan is a list of scan numbers in some files.
const scanWhere = (P: TableRef, scan: unknown) =>
  /\[\]|list/i.test(P.types.scan ?? "") ? `list_contains(${COL.scan}, ${Number(scan)})` : `${COL.scan} = ${Number(scan)}`;

function psmWhere(P: TableRef, t: PsmKey): string {
  const parts = [`${COL.peptidoform} = ${lit(t.peptidoform)}`];
  if (t.run_file_name != null && P.cols.has(COL.run_file_name)) parts.push(`${COL.run_file_name} = ${lit(String(t.run_file_name))}`);
  if (t.scan != null && P.cols.has(COL.scan)) parts.push(scanWhere(P, t.scan));
  if (t.charge != null && P.cols.has(COL.charge)) parts.push(`${COL.charge} = ${Number(t.charge)}`);
  return parts.join(" AND ");
}

/** The `file_row_number` of the first PSM matching `where`, or null. Only
 * the (narrow) filter columns are read: a filtered query that also selects
 * the peak arrays makes duckdb-wasm download them for every row group it
 * passes, gigabytes on a large streamed file. */
async function locateRow(P: TableRef, where: string): Promise<number | null> {
  const r = await runSql(`SELECT file_row_number FROM ${ident(rowsView(P.table))} WHERE ${where} LIMIT 1`, 1);
  const n = r.rows[0]?.[0];
  return n == null ? null : Number(n);
}

/** Columns `cols` of the row at `row` (one row group is read). */
async function rowAt(P: TableRef, cols: string[], row: number): Promise<Record<string, any> | undefined> {
  const r = await runSql(`SELECT ${cols.map(ident).join(", ")} FROM ${ident(rowsView(P.table))} WHERE file_row_number = ${Math.trunc(row)}`, 1);
  return objects(r)[0];
}

/** Read `cols` of the PSM: by its row number when known and still the same
 * peptidoform, otherwise found first with the narrow columns only. */
async function psmColumns(P: TableRef, cols: string[], peptidoform: string, row: number | null | undefined, where: () => string) {
  const want = cols.includes(COL.peptidoform) ? cols : [...cols, COL.peptidoform];
  if (row != null && Number.isFinite(Number(row))) {
    const hit = await rowAt(P, want, Number(row));
    if (hit && hit.peptidoform === peptidoform) return hit;
  }
  const n = await locateRow(P, where());
  return n == null ? undefined : rowAt(P, want, n);
}

/** The PSM's row (identification, scores, cv_params and peak arrays);
 * undefined when the psm view has no such PSM. */
export async function psmRecord(P: TableRef, t: PsmKey): Promise<Record<string, any> | undefined> {
  const want = SPECTRUM_COLUMNS.filter((c) => P.cols.has(c));
  return psmColumns(P, want, t.peptidoform, t.row, () => psmWhere(P, t));
}

/** Up to 201 PSMs of the peptidoform, best PEP first: run, scan, charge, PEP
 * and their row number. */
export async function peptidoformPsms(P: TableRef, peptidoform: string): Promise<Record<string, any>[]> {
  const want = [COL.run_file_name, COL.scan, COL.charge, COL.posterior_error_probability].filter((c) => P.cols.has(c));
  const order = P.cols.has(COL.posterior_error_probability) ? ` ORDER BY ${COL.posterior_error_probability}` : "";
  const r = await runSql(
    `SELECT ${[...want.map(ident), "file_row_number"].join(", ")} FROM ${ident(rowsView(P.table))} WHERE ${COL.peptidoform} = ${lit(peptidoform)}${order} LIMIT 201`,
    201
  );
  return objects(r);
}

/** Peak arrays and charge of the peptidoform's PSM `o` (a row of
 * peptidoformPsms); undefined when the view has no such row. */
export async function psmPeaks(P: TableRef, peptidoform: string, o: Record<string, any>): Promise<Record<string, any> | undefined> {
  const sc = Array.isArray(o.scan) ? o.scan[0] : o.scan;
  const where = () => {
    const parts = [`${COL.peptidoform} = ${lit(peptidoform)}`];
    if (o.run_file_name != null) parts.push(`${COL.run_file_name} = ${lit(String(o.run_file_name))}`);
    if (sc != null) parts.push(scanWhere(P, sc));
    if (o.charge != null) parts.push(`${COL.charge} = ${Number(o.charge)}`);
    return parts.join(" AND ");
  };
  const cols = [COL.mz_array, COL.intensity_array, COL.charge].filter((c) => P.cols.has(c));
  return psmColumns(P, cols, peptidoform, o.file_row_number == null ? null : Number(o.file_row_number), where);
}

/** "run · scan N · z+ · PEP x" for a PSM row. */
export function psmLabel(r: Record<string, any>): string {
  const sc = Array.isArray(r.scan) ? r.scan[0] : r.scan;
  const parts = [r.run_file_name ? String(r.run_file_name) : "", sc != null ? `scan ${sc}` : "", r.charge != null ? `${r.charge}+` : ""];
  if (r.posterior_error_probability != null) parts.push(`PEP ${Number(r.posterior_error_probability).toExponential(1)}`);
  return parts.filter(Boolean).join(" · ");
}
