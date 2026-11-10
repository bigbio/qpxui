import * as duckdb from "@duckdb/duckdb-wasm";
import { reactive } from "vue";
import { api, type ViewInfo } from "./api";
import { CANCELLED, engineErrorText } from "./errors";
import { logQuery } from "./log";

// Bundle the engine and workers here; predev/prebuild also fetch the signed
// Parquet extension into public/, so local queries need no runtime downloads.
// No `coi` (threads) bundle: it needs a cross-origin isolated page, which the
// app never is, so selectBundle would never pick it.
import duckdb_wasm_mvp from "@duckdb/duckdb-wasm/dist/duckdb-mvp.wasm?url";
import mvp_worker from "@duckdb/duckdb-wasm/dist/duckdb-browser-mvp.worker.js?url";
import duckdb_wasm_eh from "@duckdb/duckdb-wasm/dist/duckdb-eh.wasm?url";
import eh_worker from "@duckdb/duckdb-wasm/dist/duckdb-browser-eh.worker.js?url";

const MANUAL_BUNDLES: duckdb.DuckDBBundles = {
  mvp: { mainModule: duckdb_wasm_mvp, mainWorker: mvp_worker },
  eh: { mainModule: duckdb_wasm_eh, mainWorker: eh_worker },
};

let db: duckdb.AsyncDuckDB | null = null;
let conn: duckdb.AsyncDuckDBConnection | null = null;

let initPromise: Promise<void> | null = null;
/** Engine queries and registrations still running, and the engine they run on. */
let pending = 0;
let generation = 0;
/** Rejections for in-flight `tracked` calls, so abandonment settles them. */
const trackedCancels = new Set<(e: unknown) => void>();

async function tracked<T>(fn: () => Promise<T>): Promise<T> {
  const gen = generation;
  pending++;
  let rejectFn: (e: unknown) => void = () => {};
  // `cancelled` settles as soon as abandonQueries() runs: a query that was
  // running when the worker was killed would otherwise never resolve (duckdb-wasm
  // terminate() leaves pending requests hanging forever), leaving loading flags
  // stuck. Raced against the real work so a normal result still wins.
  const cancelled = new Promise<never>((_, reject) => {
    rejectFn = reject;
    trackedCancels.add(reject);
  });
  try {
    return await Promise.race([fn(), cancelled]);
  } finally {
    trackedCancels.delete(rejectFn);
    if (gen === generation) pending--;
  }
}

/** Stop whatever the engine is doing (duckdb-wasm runs one query at a time, so
 * a long scan of the previous dataset would hold up the next one). Kills the
 * worker when something is running; the next query starts a fresh engine,
 * with no tables. Returns true when it restarted, so callers drop cached
 * results. */
export function abandonQueries(): boolean {
  if (!pending || !db) return false;
  const old = db;
  db = null;
  conn = null;
  initPromise = null;
  pending = 0;
  generation++;
  registered.clear();
  const toCancel = Array.from(trackedCancels);
  trackedCancels.clear();
  for (const r of toCancel) r(new Error(CANCELLED));
  void old.terminate().catch(() => {});
  return true;
}

// Concurrent callers share one initialisation, so nobody sees a half-ready db.
export function init(): Promise<void> {
  if (!initPromise) {
    initPromise = doInit();
    initPromise.catch(() => (initPromise = null));
  }
  return initPromise;
}

async function doInit() {
  const bundle = await duckdb.selectBundle(MANUAL_BUNDLES);
  const worker = new Worker(bundle.mainWorker!);
  const logger = new duckdb.ConsoleLogger(duckdb.LogLevel.WARNING);
  const d = new duckdb.AsyncDuckDB(logger, worker);
  await d.instantiate(bundle.mainModule, bundle.pthreadWorker);
  // Remote (streamed) files must be read with range requests only. With the
  // defaults, duckdb-wasm may fall back to GETting the whole object.
  await d.open({
    filesystem: { reliableHeadRequests: true, allowFullHTTPReads: false, forceFullHTTPReads: false },
  });
  conn = await d.connect();
  const repository = new URL("/duckdb-extensions", window.location.href).href;
  await conn.query(`SET custom_extension_repository = '${repository}'`);
  // Prefetch whole column chunks of a row group in one go instead of many
  // small reads: far fewer round trips for streamed (remote) parquet files.
  await conn.query("SET prefetch_all_parquet_files = true");
  db = d;
  if (import.meta.env.DEV) (window as any).__duck = { db: d, conn };
}

function safeName(name: string): string {
  // DuckDB identifiers must be quoted; a plain name also acts as the file key.
  const base = name.replace(/\.([a-z0-9]+)$/i, ""); // strip ".parquet"
  return base.replace(/[^\w-]/g, "_");
}

/** Register a view's parquet file and expose it as a DuckDB table.
 *
 * Every view is read lazily through the Rust backend's 127.0.0.1 range proxy:
 * local files (and files downloaded to the cache) from disk, `s3://…` views
 * from S3 with the user's credentials, and portal files over HTTPS. DuckDB's
 * HTTP file system asks it only for the ranges a query needs (parquet footer,
 * then the touched column chunks), so huge files open instantly, nothing is
 * downloaded up front, and the bucket needs no CORS.
 */
/** Table name per view path. Two folders of a collection can hold files of the
 * same name: the later one gets a numbered table instead of replacing the
 * first one's. */
const tableNames = new Map<string, string>();

/** Paths of the views registered as tables in the running engine (ticked in
 * the sidebar). */
export const registered = reactive(new Set<string>());

export function tableName(view: ViewInfo): string {
  let name = tableNames.get(view.path);
  if (!name) {
    const base = safeName(view.name);
    const taken = new Set(tableNames.values());
    name = base;
    for (let i = 2; taken.has(name); i++) name = `${base}_${i}`;
    tableNames.set(view.path, name);
  }
  return name;
}

/** A new source: its views may take the names of the previous one's, and
 * are registered again. */
export function forgetTables(): void {
  tableNames.clear();
  registered.clear();
}

export async function ensureViewReady(view: ViewInfo): Promise<string> {
  // Named before any await, so concurrent registrations can't pick one name.
  const table = tableName(view);
  await init();
  if (registered.has(view.path)) return table;
  const gen = generation;
  await tracked(() => register(view, table));
  // The engine was restarted meanwhile: the table belongs to the old one.
  if (gen !== generation) throw new Error(CANCELLED);
  registered.add(view.path);
  return table;
}

async function register(view: ViewInfo, table: string): Promise<void> {
  // Bind this engine: a restart meanwhile must not touch the new one.
  const { db, conn } = await initDb();
  const handles = view.path.startsWith("s3://")
    ? await api.streamS3Urls(view.path.slice(5).split("/")[0], [view.path])
    : /^https?:\/\//.test(view.path)
      ? await api.streamHttpUrls([view.path])
      : await api.streamLocalFiles([view.path]);
  const fileKey = `${table}.parquet`;
  // Range requests only (allowFullHTTPReads is off), but with directIO =
  // false duckdb-wasm keeps the ranges it read, so later queries over the
  // same columns don't go back to the network.
  await db.registerFileURL(fileKey, handles[view.path].url, duckdb.DuckDBDataProtocol.HTTP, false);
  await conn.query(
    `CREATE OR REPLACE VIEW "${table}" AS SELECT * FROM read_parquet('${fileKey}')`
  );
}

async function initDb(): Promise<{ db: duckdb.AsyncDuckDB; conn: duckdb.AsyncDuckDBConnection }> {
  await init();
  return { db: db!, conn: conn! };
}

export type Cell = string | number | bigint | boolean | null;

export interface QueryResult {
  columns: string[];
  rows: (Cell | object | unknown[])[][];
  rowCount: number;
  truncated: boolean;
}

/**
 * Is this an Apache Arrow Vector (or a chunk of one)? duckdb-wasm hands back
 * Vector objects for STRUCT / LIST columns that were not materialised into
 * plain JS by `.toArray()`. They expose a `.toArray()` we can call to get
 * friendly JS values, plus `.type.typeId`.
 */
function isArrowVector(v: unknown): boolean {
  return (
    typeof v === "object" &&
    v !== null &&
    !Array.isArray(v) &&
    typeof (v as Record<string, unknown>).toArray === "function" &&
    typeof (v as { type?: { typeId?: unknown } }).type?.typeId === "number"
  );
}

function normalize(v: unknown): unknown {
  if (v === null || v === undefined) return null;
  const t = typeof v;
  // 64-bit integers past 2^53 (QPX's hashed ids) stay BigInt: as a Number
  // they would lose their last digits.
  if (t === "bigint") return Number.isSafeInteger(Number(v)) ? Number(v) : v;
  if (t === "string" || t === "number" || t === "boolean") return v;
  if (Array.isArray(v)) return v.map(normalize);
  // Arrow Vector / chunk -> collapse to a friendly JS array of values, then
  // normalise recursively (a LIST of STRUCT becomes [{...}, {...}], not a dump
  // of the Vector's internal buffers).
  if (isArrowVector(v)) return normalize((v as { toArray: () => unknown }).toArray());
  if (v instanceof Map) {
    const out: Record<string, unknown> = {};
    for (const [k, val] of v) out[String(k)] = normalize(val);
    return out;
  }
  // Arrow rows (StructRow / MapRow) expose toJSON() with the real fields.
  if (typeof (v as { toJSON?: unknown }).toJSON === "function" && !(v instanceof Date)) {
    const j = (v as { toJSON: () => unknown }).toJSON();
    if (j !== v) return normalize(j);
  }
  // Typed arrays (Float64Array, Int32Array, …) have numeric-ish elements.
  if (ArrayBuffer.isView(v)) {
    return Array.from(v as unknown as ArrayLike<number>);
  }
  if (t === "object") {
    // Keep STRUCT / MAP values as plain objects so views can render them
    // nicely, only normalising bigints to numbers recursively.
    const out: Record<string, unknown> = {};
    for (const [k, val] of Object.entries(v as Record<string, unknown>)) {
      out[k] = normalize(val);
    }
    return out;
  }
  return String(v);
}

/** Dev builds keep a log of query timings in `window.__qlog`. */
function recordQuery(sql: string, ms: number, rows: number, error?: string) {
  logQuery(sql, ms, rows, error);
  if (import.meta.env.DEV) {
    const w = window as any;
    (w.__qlog ??= []).push({ ms: Math.round(ms), rows, sql: sql.replace(/\s+/g, " ").trim() });
  }
}

type QueryResultType = Awaited<ReturnType<duckdb.AsyncDuckDBConnection["query"]>>;

/**
 * A result's rows as arrays of display values, read column by column: Arrow's
 * row objects key values by column name and keep only one of a shared name.
 */
export function readRows(table: QueryResultType): QueryResult["rows"] {
  const columns = table.schema.fields.map((_, j) => table.getChildAt(j)!);
  return Array.from({ length: table.numRows }, (_, i) => columns.map((c) => normalize(c.get(i)) as never));
}

export async function runSql(
  sql: string,
  maxRows = 5000
): Promise<QueryResult> {
  await init();
  let res: QueryResultType;
  const t0 = performance.now();
  const w = window as any;
  if (import.meta.env.DEV) w.__qpending = (w.__qpending ?? 0) + 1;
  try {
    const c = conn!;
    res = await tracked(() => c.query(sql));
  } catch (e) {
    const text = engineErrorText(e);
    recordQuery(sql, performance.now() - t0, 0, text);
    throw new Error(text);
  } finally {
    if (import.meta.env.DEV) w.__qpending--;
  }
  recordQuery(sql, performance.now() - t0, res.numRows);
  const truncated = res.numRows > maxRows;
  const table = truncated ? res.slice(0, maxRows) : res;
  // Columns come from the schema, so an empty result keeps them, and values
  // are read by position, so duplicate column names keep their own values.
  const columns = table.schema.fields.map((f) => f.name);
  const rows = readRows(table);
  return { columns, rows, rowCount: rows.length, truncated };
}

export async function countRows(table: string): Promise<number> {
  const r = await runSql(`SELECT COUNT(*) AS n FROM "${table}"`, 1);
  return Number(r.rows[0][0]);
}

export interface ColumnInfo {
  name: string;
  type: string;
}

/** Read column names + DuckDB types via DESCRIBE. */
export async function describeTable(table: string): Promise<ColumnInfo[]> {
  const r = await runSql(`DESCRIBE "${table}"`);
  // DESCRIBE's first two columns: column_name, column_type.
  return r.rows.map(([name, type]) => ({ name: String(name), type: String(type) }));
}
