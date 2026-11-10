import { afterEach, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
import { invoke } from "@tauri-apps/api/core";
import { logQuery, setDetailedLogging } from "./log";

const sent = () => vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "log_write").map(([, a]) => a as { level: string; message: string });

afterEach(() => vi.mocked(invoke).mockClear());

it("logs slow queries and failures, and fast ones only when detailed", async () => {
  vi.spyOn(console, "info").mockImplementation(() => {});
  vi.spyOn(console, "warn").mockImplementation(() => {});
  vi.spyOn(console, "debug").mockImplementation(() => {});
  logQuery("SELECT 1", 5, 1);
  expect(sent()).toEqual([]);
  logQuery("SELECT  count(*)\n FROM t", 2500, 1);
  logQuery("SELECT x FROM t", 12, 0, "Binder Error: x");
  expect(sent()).toEqual([
    { level: "info", message: "slow query: 2500 ms, 1 rows | SELECT count(*) FROM t" },
    { level: "warn", message: "query failed after 12 ms: Binder Error: x | SELECT x FROM t" },
  ]);
  vi.mocked(invoke).mockClear();
  await setDetailedLogging(true);
  logQuery("SELECT 1", 5, 1);
  expect(sent()).toEqual([{ level: "debug", message: "query: 5 ms, 1 rows | SELECT 1" }]);
});
