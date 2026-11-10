// The frontend's side of the app log (see src-tauri/src/logging.rs): lines go
// to the browser console and to the log file the Diagnostics tab shows.
import { invoke } from "@tauri-apps/api/core";

type Level = "error" | "warn" | "info" | "debug";

/** Whether detailed (debug) lines are sent; mirrors the backend's level. */
let detailed = false;

function write(level: Level, message: string) {
  if (level === "debug" && !detailed) return;
  (level === "debug" ? console.debug : console[level])(`[qpxui] ${message}`);
  // Logging must never break the app (e.g. in tests, outside Tauri).
  try {
    void invoke("log_write", { level, message }).catch(() => {});
  } catch {
    /* not in Tauri */
  }
}

export const log = {
  error: (m: string) => write("error", m),
  warn: (m: string) => write("warn", m),
  info: (m: string) => write("info", m),
  debug: (m: string) => write("debug", m),
};

/** Turn detailed logging on or off, here and in the backend. */
export async function setDetailedLogging(on: boolean): Promise<void> {
  detailed = on;
  await invoke("log_set_detailed", { detailed: on });
}

/** Queries slower than this are logged even without detailed logging. */
export const SLOW_QUERY_MS = 1000;

/** One line about a finished query: every query when detailed, slow ones
 * always. The SQL is shortened to one line. */
export function logQuery(sql: string, ms: number, rows: number, error?: string) {
  const text = sql.replace(/\s+/g, " ").trim();
  const short = text.length > 600 ? `${text.slice(0, 600)}…` : text;
  const took = `${Math.round(ms)} ms`;
  if (error) log.warn(`query failed after ${took}: ${error} | ${short}`);
  else if (ms >= SLOW_QUERY_MS) log.info(`slow query: ${took}, ${rows} rows | ${short}`);
  else log.debug(`query: ${took}, ${rows} rows | ${short}`);
}

/** Send uncaught errors and unhandled promise rejections to the log. */
export function installErrorLogging() {
  window.addEventListener("error", (e) => log.error(`uncaught: ${e.message} at ${e.filename}:${e.lineno}`));
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason;
    log.error(`unhandled rejection: ${r?.stack ?? r?.message ?? String(r)}`);
  });
}

export interface LogInfo {
  path: string | null;
  detailed: boolean;
  text: string;
}

/** The log file's path, level and last lines. */
export function readLog(): Promise<LogInfo> {
  return invoke<LogInfo>("log_read");
}
