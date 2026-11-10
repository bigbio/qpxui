// The dataset report's header: project metadata from the dataset view, the
// KPI counts, and the instruments, enzymes and organisms of the dataset.
import type { DatasetInfo } from "../api";
import { cachedSql, evidence, ident, objects, proteinExpr, targetWhere, type DatasetTables, type TableRef } from "../report";
import { COL } from "../qpx";

/** Project metadata (accession, title, description, PubMed id, software) from
 * the dataset view. */
export async function datasetMeta(ds: DatasetInfo, dataset: TableRef): Promise<Record<string, any>> {
  const sel = [COL.project_accession, COL.project_title, COL.project_description, COL.pubmed_id, COL.software_name, COL.software_version, COL.creation_date]
    .filter((c) => dataset.cols.has(c));
  if (!sel.length) return {};
  const r = await cachedSql(ds, "hdr.dataset", `SELECT ${sel.join(", ")} FROM ${ident(dataset.table)} LIMIT 1`);
  return objects(r)[0] ?? {};
}

/** One header count. `distinct` counts read whole columns, `rows` counts
 * come from the parquet footer on large streamed views, `count` counts are
 * small views, always queried. */
export interface KpiSource {
  key: string;
  label: string;
  title?: string;
  kind: "distinct" | "rows" | "count";
  ref: TableRef;
  cacheKey: string;
  sql: string;
}

/** The header counts the dataset's views support, in display order. */
export function kpiSources(t: DatasetTables): KpiSource[] {
  const out: KpiSource[] = [];
  const ev = evidence(t);
  const rows = (ref: TableRef, key: string, label: string, cacheKey: string): KpiSource =>
    ({ key, label, kind: "rows", ref, cacheKey, sql: `SELECT count(*) FROM ${ident(ref.table)}` });
  if (t.pg) {
    out.push({ key: "pg", label: "Protein groups", title: "Distinct anchor proteins (target) in the pg view", kind: "distinct", ref: t.pg, cacheKey: "kpi.pg", sql: `SELECT count(DISTINCT ${COL.anchor_protein}) FROM ${ident(t.pg.table)} WHERE ${targetWhere(t.pg)}` });
  } else if (ev && proteinExpr(ev)) {
    out.push({ key: "pg", label: "Proteins", kind: "distinct", ref: ev, cacheKey: "kpi.prot", sql: `SELECT count(DISTINCT ${proteinExpr(ev)}) FROM ${ident(ev.table)} WHERE ${targetWhere(ev)}` });
  }
  if (ev && ev.seq !== "NULL") {
    out.push({ key: "pep", label: "Peptides", title: "Distinct target peptide sequences", kind: "distinct", ref: ev, cacheKey: "kpi.pep", sql: `SELECT count(DISTINCT ${ev.seq}) FROM ${ident(ev.table)} WHERE ${targetWhere(ev)}` });
  }
  if (t.feature) out.push(rows(t.feature, "feat", "Features", "kpi.feat"));
  if (t.psm) out.push(rows(t.psm, "psm", "PSMs", "kpi.psm"));
  if (t.run) out.push({ key: "run", label: "Runs", kind: "count", ref: t.run, cacheKey: "kpi.run", sql: `SELECT count(*) FROM ${ident(t.run.table)}` });
  else if (ev?.cols.has(COL.run_file_name)) out.push({ key: "run", label: "Runs", kind: "distinct", ref: ev, cacheKey: "kpi.run2", sql: `SELECT count(DISTINCT ${COL.run_file_name}) FROM ${ident(ev.table)}` });
  if (t.sample) out.push({ key: "smp", label: "Samples", kind: "count", ref: t.sample, cacheKey: "kpi.smp", sql: `SELECT count(*) FROM ${ident(t.sample.table)}` });
  return out;
}

/** A whole-column count the portal catalogue already has for this dataset,
 * so the header need not scan a large streamed view for it. Only counts that
 * mean the same thing: distinct proteins from the evidence view (not the pg
 * view's protein groups) and distinct peptides. */
export function catalogCount(ds: DatasetInfo, s: KpiSource): number | null {
  const c = ds.catalog;
  if (!c || s.kind !== "distinct") return null;
  const v = s.cacheKey === "kpi.prot" ? c.proteins : s.cacheKey === "kpi.pep" ? c.peptides : null;
  return typeof v === "number" && v > 0 ? v : null;
}

/** A count's value; failures are shown beside that count in the header. */
export async function countValue(ds: DatasetInfo, s: KpiSource): Promise<number> {
  const r = await cachedSql(ds, s.cacheKey, s.sql);
  return Number(r.rows[0]?.[0] ?? 0);
}

/** Instruments and enzymes across the runs. */
export async function runInstrumentsEnzymes(ds: DatasetInfo, run: TableRef): Promise<{ instruments: string[]; enzymes: string[] }> {
  const c = run.cols;
  const parts = [
    c.has(COL.instrument) ? `list_sort(list_distinct(list(${COL.instrument}))) AS instruments` : "[]::VARCHAR[] AS instruments",
    c.has(COL.enzymes) ? `list_sort(list_distinct(flatten(list(${COL.enzymes})))) AS enzymes` : "[]::VARCHAR[] AS enzymes",
  ];
  const r = objects(await cachedSql(ds, "hdr.run", `SELECT ${parts.join(", ")} FROM ${ident(run.table)}`))[0];
  return { instruments: (r?.instruments ?? []).filter(Boolean), enzymes: (r?.enzymes ?? []).filter(Boolean) };
}

/** Organisms of the samples. */
export async function sampleOrganisms(ds: DatasetInfo, sample: TableRef): Promise<string[]> {
  const r = await cachedSql(ds, "hdr.org", `SELECT DISTINCT ${COL.organism} FROM ${ident(sample.table)} WHERE ${COL.organism} IS NOT NULL ORDER BY 1`);
  return r.rows.map((x) => String(x[0]));
}
