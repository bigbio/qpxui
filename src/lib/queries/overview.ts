// The Overview tab: whole-dataset distributions of the evidence view
// (features, or PSMs), one query per chart.
import type { DatasetInfo } from "../api";
import type { ChartSpec } from "../charts";
import {
  cachedSql,
  histogramData,
  histogramSql,
  ident,
  intensitySumExpr,
  log10Expr,
  proteinExpr,
  targetWhere,
  type TableRef,
} from "../report";
import { COL } from "../qpx";

/** Charts in the order they are computed: narrow numeric columns first, so
 * the first charts of a large streamed dataset appear quickly; the per-run,
 * charge and missed-cleavage counts share one scan; the charts over whole
 * sequence columns come last. (The tab lays the cards out in its own order.) */
export const OVERVIEW_CHARTS = ["charge", "mc", "perRun", "pep", "qvalue", "rt", "ppm", "intensity", "length", "ppg"] as const;
export type OverviewChart = (typeof OVERVIEW_CHARTS)[number];

/** Evidence-view columns the charts read (for the large-view gate). */
export const OVERVIEW_COLUMNS = [COL.is_decoy, COL.run_file_name, COL.charge, COL.sequence, COL.peptidoform, COL.missed_cleavages, COL.anchor_protein,
  COL.protein_accessions, COL.pg_accessions, COL.mass_error_ppm, COL.rt, COL.intensities, COL.posterior_error_probability, COL.peptide_qvalue];

/** The columns of `countColumns`, in query order. */
type CountColumn = typeof COL.run_file_name | typeof COL.charge | typeof COL.missed_cleavages;

/** One scan for several counts charts: target rows per value of each of
 * `cols`, as GROUPING SETS. Row: the grouped column's name, its value, count. */
export function countsSql(table: string, where: string, cols: CountColumn[]): string {
  const sets = cols.map((c) => `(${c})`).join(", ");
  const which = cols
    .map((c) => `WHEN grouping(${c}) = 0 THEN '${c}'`)
    .join(" ");
  const value = `CASE ${cols.map((c) => `WHEN grouping(${c}) = 0 THEN ${c}::VARCHAR`).join(" ")} END`;
  return `SELECT CASE ${which} END AS col, ${value} AS val, count(*) AS n
FROM ${ident(table)} WHERE ${where}
GROUP BY GROUPING SETS (${sets})`;
}

/** A loader per chart the evidence view `ev` has the columns for; `evName`
 * names its rows ("features" or "PSMs"). */
export function overviewCharts(
  ds: DatasetInfo,
  ev: TableRef,
  evName: string
): Partial<Record<OverviewChart, () => Promise<ChartSpec | null>>> {
  const T = ident(ev.table);
  const W = targetWhere(ev);
  const has = (c: string) => ev.cols.has(c);
  const prot = proteinExpr(ev);
  const isum = intensitySumExpr(ev);

  // Run, charge and missed-cleavage counts come from one shared scan.
  const countCols = ([COL.run_file_name, COL.charge, COL.missed_cleavages] as CountColumn[]).filter(has);
  let shared: Promise<Map<string, [string | null, number][]>> | null = null;
  function sharedCounts(col: CountColumn): Promise<[string | null, number][]> {
    shared ??= cachedSql(ds, "ov.counts", countsSql(ev.table, W, countCols)).then((r) => {
      const by = new Map<string, [string | null, number][]>();
      for (const [c, v, n] of r.rows) {
        const list = by.get(String(c)) ?? [];
        list.push([v === null || v === undefined ? null : String(v), Number(n)]);
        by.set(String(c), list);
      }
      return by;
    });
    return shared.then((by) => by.get(col) ?? []);
  }
  /** Bars by value (numeric values in numeric order, ∅ last). */
  async function valueCounts(col: CountColumn, xName: string): Promise<ChartSpec> {
    const rows = [...(await sharedCounts(col))].sort((a, b) =>
      a[0] === null ? 1 : b[0] === null ? -1 : Number(a[0]) - Number(b[0])
    );
    return {
      kind: "bar",
      labels: rows.map((x) => x[0] ?? "∅"),
      series: [{ name: evName, values: rows.map((x) => x[1]) }],
      xName,
    };
  }

  async function counts(key: string, sql: string, xName: string, yName = evName): Promise<ChartSpec> {
    const r = await cachedSql(ds, key, sql);
    return {
      kind: "bar",
      labels: r.rows.map((x) => String(x[0] ?? "∅")),
      series: [{ name: yName, values: r.rows.map((x) => Number(x[1])) }],
      xName,
    };
  }

  async function hist(key: string, expr: string, where: string, xName: string, yName = evName, robust = false): Promise<ChartSpec | null> {
    const r = await cachedSql(ds, key, histogramSql(ev.table, expr, where, 40, robust));
    const h = histogramData(r);
    // A distribution over the whole dataset: a filled line, as on the portal.
    return { kind: "line", zeroBased: true, labels: h.labels, series: [{ name: yName, values: h.values }], notes: h.ranges.map((range) => `Range: ${range}`), xName };
  }

  return {
    perRun: has(COL.run_file_name)
      ? async () => {
          const rows = [...(await sharedCounts(COL.run_file_name))].sort((a, b) => b[1] - a[1]);
          return {
            kind: "line",
            labels: rows.map((x) => x[0] ?? "∅"),
            series: [{ name: evName, values: rows.map((x) => x[1]) }],
            xName: `runs (${rows.length}), most first`,
            hideLabels: rows.length > 30,
            zeroBased: true,
          };
        }
      : undefined,
    charge: has(COL.charge) ? () => valueCounts(COL.charge, "charge") : undefined,
    length: ev.seq !== "NULL"
      ? () =>
          counts(
            "ov.length",
            `SELECT length(${ev.seq}) AS len, count(*) FROM ${T} WHERE ${W} AND ${ev.seq} IS NOT NULL GROUP BY 1 ORDER BY 1`,
            "residues"
          )
      : undefined,
    mc: has(COL.missed_cleavages) ? () => valueCounts(COL.missed_cleavages, "missed cleavages") : undefined,
    ppg: prot && ev.seq !== "NULL"
      ? async () => {
          const s = await counts(
            "ov.ppg",
            `SELECT least(n, 20) AS k, count(*) FROM (
               SELECT ${prot} AS p, count(DISTINCT ${ev.seq}) AS n FROM ${T}
               WHERE ${W} AND ${prot} IS NOT NULL GROUP BY 1
             ) GROUP BY 1 ORDER BY 1`,
            "peptides (20 = 20 or more)",
            "protein groups"
          );
          s.labels = s.labels.map((l) => (l === "20" ? "20+" : l));
          return s;
        }
      : undefined,
    ppm: has(COL.mass_error_ppm) ? () => hist("ov.ppm", COL.mass_error_ppm, W, "ppm", evName, true) : undefined,
    rt: has(COL.rt) ? () => hist("ov.rt", `${COL.rt} / 60.0`, W, "minutes") : undefined,
    intensity: isum ? () => hist("ov.int", log10Expr(isum), W, "log10 intensity") : undefined,
    pep: has(COL.posterior_error_probability)
      ? () => hist("ov.pep", log10Expr(COL.posterior_error_probability), W, "log10 PEP", evName)
      : undefined,
    qvalue: has(COL.peptide_qvalue)
      ? () => hist("ov.q", COL.peptide_qvalue, W, "peptide q-value", evName)
      : undefined,
  };
}
