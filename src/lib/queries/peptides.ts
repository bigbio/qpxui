// The Peptidoforms tab: peptidoform search over the evidence view and one
// peptidoform's per-run evidence (features, or PSMs when they carry spectra).
import { rowsView, runSql } from "../duck";
import { ident, intensitySumExpr, likePattern, lit, objects, targetWhere, type TableRef } from "../report";
import { COL } from "../qpx";

/** Rows a search lists; one more is read to tell that there are more. */
export const PEPTIDOFORM_LIMIT = 500;

export interface PeptidoformFilter {
  /** Sequence text (non-letters dropped) or peptidoform text. */
  q: string;
  mode: "contains" | "exact" | "prefix";
  /** Modification name, "" for any. */
  mod: string;
  minCharge: number | null;
}

/** WHERE clause for target rows of `ev` matching the filter. */
export function peptidoformWhere(ev: TableRef, f: PeptidoformFilter): string {
  const parts = [targetWhere(ev)];
  // Strip anything that is not an amino-acid letter, so "pep tide" / lower case work.
  const s = f.q.trim().toUpperCase();
  if (s) {
    const aa = s.replace(/[^A-Z]/g, "");
    const seqCond =
      f.mode === "exact" ? `${ev.seq} = ${lit(aa)}`
      : f.mode === "prefix" ? `starts_with(${ev.seq}, ${lit(aa)})`
      : `contains(${ev.seq}, ${lit(aa)})`;
    // Also match peptidoform text (e.g. "M[Oxidation]" or "UNIMOD:21").
    const pfCond = ev.cols.has(COL.peptidoform) && /[[\]:+]/.test(f.q) ? ` OR ${COL.peptidoform} ILIKE ${likePattern(f.q.trim())}` : "";
    parts.push(`(${aa ? seqCond : "FALSE"}${pfCond})`);
  }
  if (f.mod && ev.cols.has(COL.modifications)) {
    parts.push(`list_contains(list_transform(${COL.modifications}, x -> x.name), ${lit(f.mod)})`);
  }
  if (f.minCharge && ev.cols.has(COL.charge)) parts.push(`${COL.charge} >= ${Number(f.minCharge)}`);
  return parts.join(" AND ");
}

/** Peptidoforms matching the filter, most evidence rows first, with their
 * protein (`prot` of `ev`), genes, charges, runs, best PEP and mean log10
 * intensity; the first PEPTIDOFORM_LIMIT of them. */
export async function searchPeptidoforms(
  ev: TableRef,
  prot: string | null,
  f: PeptidoformFilter
): Promise<{ rows: Record<string, any>[]; truncated: boolean }> {
  const has = (c: string) => ev.cols.has(c);
  const isum = intensitySumExpr(ev);
  const pf = has(COL.peptidoform) ? COL.peptidoform : ev.seq;
  const sel = [
    `${pf} AS peptidoform`,
    `any_value(${ev.seq}) AS sequence`,
    has(COL.modifications) ? `any_value(${COL.modifications}) AS modifications` : null,
    prot ? `any_value(${prot}) AS protein` : null,
    has(COL.gg_names) ? `any_value(${COL.gg_names}) AS genes` : null,
    has(COL.charge) ? `list_sort(list_distinct(list(${COL.charge}))) AS charges` : null,
    "count(*)::BIGINT AS n_ev",
    has(COL.run_file_name) ? `count(DISTINCT ${COL.run_file_name})::BIGINT AS runs` : null,
    has(COL.posterior_error_probability) ? `min(${COL.posterior_error_probability}) AS best_pep` : null,
    isum ? `avg(CASE WHEN ${isum} > 0 THEN log10(${isum}) END) AS log_int` : null,
  ].filter(Boolean);
  const r = await runSql(
    `SELECT ${sel.join(", ")} FROM ${ident(ev.table)} WHERE ${peptidoformWhere(ev, f)}
       GROUP BY 1 ORDER BY n_ev DESC, ${COL.peptidoform} LIMIT ${PEPTIDOFORM_LIMIT + 1}`,
    PEPTIDOFORM_LIMIT + 1
  );
  const out = objects(r);
  return { rows: out.slice(0, PEPTIDOFORM_LIMIT), truncated: out.length > PEPTIDOFORM_LIMIT };
}

/** The peptidoform's target rows in `E` (run, scan, charge, RT, m/z, ppm,
 * PEP, log10 intensity), by run and PEP; at most 5000. */
export async function peptidoformEvidence(E: TableRef, peptidoform: string): Promise<Record<string, any>[]> {
  const has = (c: string) => E.cols.has(c);
  const isum = intensitySumExpr(E);
  const pfCol = has(COL.peptidoform) ? COL.peptidoform : E.seq;
  const scanList = /\[\]|list/i.test(E.types.scan ?? "");
  const sel = [
    has(COL.run_file_name) ? COL.run_file_name : null,
    has(COL.scan) ? (scanList ? `${COL.scan}[1] AS scan_no` : `${COL.scan} AS scan_no`) : null,
    has(COL.charge) ? COL.charge : null,
    has(COL.rt) ? `${COL.rt} / 60.0 AS rt_min` : null,
    has(COL.observed_mz) ? COL.observed_mz : null,
    has(COL.mass_error_ppm) ? COL.mass_error_ppm : null,
    has(COL.posterior_error_probability) ? COL.posterior_error_probability : null,
    isum ? `CASE WHEN ${isum} > 0 THEN log10(${isum}) END AS log_int` : null,
    // Where the row is in the file, so the spectrum viewer reads it directly.
    "file_row_number",
  ].filter(Boolean);
  const r = await runSql(
    `SELECT ${sel.join(", ")} FROM ${ident(rowsView(E.table))} WHERE ${targetWhere(E)} AND ${pfCol} = ${lit(peptidoform)}
       ORDER BY ${has(COL.run_file_name) ? COL.run_file_name : "1"}${has(COL.posterior_error_probability) ? `, ${COL.posterior_error_probability}` : ""} LIMIT 5000`
  );
  return objects(r);
}
