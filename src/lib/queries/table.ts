// The browse table: one page of a view's rows, filtered by a WHERE clause
// and sorted by a column, with the per-peak arrays of a psm view last.
import { runSql, type QueryResult } from "../duck";
import { ident, lit } from "../report";
import { COL } from "../qpx";

/** Per-peak arrays of a QPX psm row. They hold hundreds of values each, so
 * the table shows them as "N peaks", after the other columns. */
export const PEAK_COLS = new Set<string>([COL.mz_array, COL.intensity_array, COL.charge_array, COL.ion_type_array, COL.ion_mobility_array]);

export const isPeakColumn = (c: string) => PEAK_COLS.has(c.toLowerCase());

/**
 * Heavy or rarely used columns the Browse table skips by default (the SELECT
 * never reads them, so for streamed files their chunks aren't fetched). They
 * are loaded on demand: per view from the table's "Columns" menu, or for a
 * single PSM from the spectrum viewer. `additional_scores` and
 * `additional_intensities` are name-value list columns not in `COL`.
 */
export interface OptionalColumn {
  /** Exact column name as it appears in the parquet file. */
  col: string;
  /** Short human label for the settings panel. */
  label: string;
}
export const OPTIONAL_COLUMNS: OptionalColumn[] = [
  { col: COL.mz_array, label: "Peak array: m/z" },
  { col: COL.intensity_array, label: "Peak array: intensity" },
  { col: COL.charge_array, label: "Peak array: charge" },
  { col: COL.ion_type_array, label: "Peak array: ion type" },
  { col: COL.ion_mobility_array, label: "Peak array: ion mobility" },
  { col: COL.cv_params, label: "cv_params (CV-controlled parameters)" },
  { col: "additional_scores", label: "additional_scores" },
  { col: "additional_intensities", label: "additional_intensities" },
];
const OPTIONAL_SET = new Set(OPTIONAL_COLUMNS.map((o) => o.col.toLowerCase()));
export const isOptionalColumn = (c: string) => OPTIONAL_SET.has(c.toLowerCase());

/** The SELECT list: the plain columns, then (with `peaks`) the peak arrays. */
export function selectList(cols: string[], peaks: boolean): string {
  if (!cols.length) return "*";
  const plain = cols.filter((c) => !isPeakColumn(c));
  const pk = peaks ? cols.filter(isPeakColumn) : [];
  return [...plain, ...pk].map(ident).join(", ");
}

export interface PageRequest {
  table: string;
  /** The view's columns, in order; empty for all. */
  cols: string[];
  peaks: boolean;
  /** WHERE clause as typed. */
  where: string;
  sort: { col: string; dir: "ASC" | "DESC" } | null;
  page: number;
  pageSize: number;
}

function whereClause(where: string): string {
  return where.trim() ? ` WHERE ${where.trim()}` : "";
}

export function countSql(table: string, where: string): string {
  return `SELECT count(*) FROM ${ident(table)}${whereClause(where)}`;
}

/** Rows that match the browse filter. */
export async function tableCount(table: string, where: string): Promise<number> {
  const r = await runSql(countSql(table, where));
  return Number(r.rows[0][0]);
}

export function pageSql(r: PageRequest): string {
  const orderClause = r.sort ? ` ORDER BY ${ident(r.sort.col)} ${r.sort.dir}` : "";
  return `SELECT ${selectList(r.cols, r.peaks)} FROM ${ident(r.table)}${whereClause(r.where)}${orderClause} LIMIT ${r.pageSize} OFFSET ${r.page * r.pageSize}`;
}

/** One page of rows. */
export function tablePage(r: PageRequest): Promise<QueryResult> {
  return runSql(pageSql(r));
}

/** A WHERE condition for rows whose `col` equals a cell's value: IS NULL,
 * a string or integer-valued number literal, else the value as text. */
export function cellFilter(col: string, v: unknown): string {
  if (v == null) return `${ident(col)} IS NULL`;
  if (typeof v === "string") return `${ident(col)} = ${lit(v)}`;
  if (typeof v === "bigint" || Number.isSafeInteger(v)) return `${ident(col)} = ${v}`;
  // A decimal literal can round to a float next to the one read; cast from
  // text it is that very DOUBLE, which a FLOAT column widens to exactly.
  if (typeof v === "number" && Number.isFinite(v)) return `${ident(col)} = ${lit(String(v))}::DOUBLE`;
  return `CAST(${ident(col)} AS VARCHAR) = ${lit(String(v))}`;
}
