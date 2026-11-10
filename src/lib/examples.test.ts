import { expect, it } from "vitest";
import type { PortalDataset } from "./api";
import { EXAMPLE_GROUPS, pickExamples, type ExampleGroup } from "./examples";

const ds = (ref: string, extra: Partial<PortalDataset> = {}): PortalDataset => ({
  accession: ref.split("/")[0],
  datasetRef: ref,
  ...extra,
});

const groups: ExampleGroup[] = [
  { title: "A", intro: "", examples: [{ ref: "PXD1/r1", note: "one" }, { ref: "PXD2/r2", note: "two" }] },
  { title: "B", intro: "", examples: [{ ref: "PXD9/gone", note: "missing" }] },
];

it("finds the curated runs in registry order, dropping missing ones and empty groups", () => {
  const out = pickExamples([ds("PXD2/r2"), ds("PXD1/r1"), ds("PXD3/r3")], groups);
  expect(out.map((g) => g.group.title)).toEqual(["A"]);
  expect(out[0].items.map((i) => i.dataset.datasetRef)).toEqual(["PXD1/r1", "PXD2/r2"]);
});

it("prefers the duplicate entry that has counts", () => {
  const out = pickExamples([ds("PXD1/r1"), ds("PXD1/r1", { psmCount: 42 })], groups);
  expect(out[0].items[0].dataset.psmCount).toBe(42);
});

it("falls back to the accession only when it is unambiguous", () => {
  const single = pickExamples([ds("PXD1/new", { organism: "homo sapiens" })], groups);
  expect(single[0].items[0].dataset.datasetRef).toBe("PXD1/new");
  const many = pickExamples([ds("PXD1/x", { organism: "mus musculus" }), ds("PXD1/y", { organism: "danio rerio" })], groups);
  expect(many).toEqual([]);
});

it("curates unique refs", () => {
  const refs = EXAMPLE_GROUPS.flatMap((g) => g.examples.map((e) => e.ref));
  expect(new Set(refs).size).toBe(refs.length);
});
