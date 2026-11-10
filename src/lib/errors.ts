// Error texts shared by the query engine, the backend and the views.

/** What a cancelled download or an abandoned engine query rejects with: the
 * backend's error string, or an Error carrying it. */
export const CANCELLED = "cancelled";

export function isCancelled(e: unknown): boolean {
  return (e instanceof Error ? e.message : e) === CANCELLED;
}

/**
 * The text of a DuckDB error. A failed read of a streamed file (duckdb-wasm
 * reports those as "Range request for <url> …" or an unexpected status while
 * opening it) gets a hint, so the user knows it is usually the network or
 * credentials rather than a bug in the query.
 */
export function engineErrorText(e: unknown): string {
  const msg = String((e as any)?.message ?? e);
  if (/Range request for |Unexpected return status from server/.test(msg)) {
    return (
      `${msg}\n\n` +
      "Hint: a remote (streamed) read failed — the remote storage was slow or unreachable " +
      "(for S3, also check your AWS credentials). Try again, or use “Download to cache” " +
      "for this view; Settings → Diagnostics → Save log shows what failed."
    );
  }
  return msg;
}
