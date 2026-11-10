import { expect, it } from "vitest";
import type { DatasetInfo } from "../api";
import { catalogCount, type KpiSource } from "./header";

const src = (cacheKey: string, kind: KpiSource["kind"] = "distinct") =>
  ({ key: "k", label: "l", kind, cacheKey, sql: "", ref: {} as KpiSource["ref"] }) as KpiSource;
const ds = (catalog?: DatasetInfo["catalog"]): DatasetInfo => ({ prefix: "X", path: "x", views: [], catalog });

it("takes distinct protein and peptide counts from the portal catalogue", () => {
  const d = ds({ proteins: 18682, peptides: 252246, psmCount: 58527091 });
  expect(catalogCount(d, src("kpi.prot"))).toBe(18682);
  expect(catalogCount(d, src("kpi.pep"))).toBe(252246);
});

it("leaves counts the catalogue does not describe to the data", () => {
  const d = ds({ proteins: 18682, peptides: 0 });
  // Protein groups of the pg view are not the catalogue's proteins.
  expect(catalogCount(d, src("kpi.pg"))).toBeNull();
  // Unknown (0) and missing counts.
  expect(catalogCount(d, src("kpi.pep"))).toBeNull();
  expect(catalogCount(ds(), src("kpi.prot"))).toBeNull();
  // Row counts come from the parquet footer anyway.
  expect(catalogCount(d, src("kpi.prot", "rows"))).toBeNull();
});
