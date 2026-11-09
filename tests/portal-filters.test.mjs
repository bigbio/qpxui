import assert from "node:assert/strict";
import { test } from "vitest";
import { buildInstrumentTree, findInstrumentNode, ontologyAccession, ontologyName, selectedInstruments } from "../src/lib/ontology";
import { matchesMethod, methodTypes, enrichPortalMethods } from "../src/lib/portalMethods";

test("ontology labels preserve plain names and extract encoded names", () => {
  assert.equal(ontologyName("NT=Instrumentname;AC=MS:000001"), "Instrumentname");
  assert.equal(ontologyName("AC=MS:000001;NT=Instrumentname"), "Instrumentname");
  assert.equal(ontologyName("Instrumentname"), "Instrumentname");
  assert.equal(ontologyName(null), "");
  assert.equal(ontologyAccession("NT=Instrumentname;AC=MS:000001"), "MS:000001");
});

const terms = [
  { id: "MS:1000463", name: "instrument", parents: [] },
  { id: "MS:1", name: "Class A", parents: ["MS:1000463"] },
  { id: "MS:2", name: "Class B", parents: ["MS:1000463"] },
  { id: "MS:3", name: "Model A", parents: ["MS:1", "MS:2"] },
  { id: "MS:4", name: "Model B", parents: ["MS:1"] },
];
const modelA = "NT=Model A;AC=MS:3";
const modelB = "NT=Model B;AC=MS:4";

test("class filters include every descendant, preserving multiple inheritance", () => {
  const tree = buildInstrumentTree(terms, [modelA, modelB]);
  assert.deepEqual(new Set(findInstrumentNode(tree, "MS:1").instruments), new Set([modelA, modelB]));
  assert.deepEqual(findInstrumentNode(tree, "MS:2").instruments, [modelA]);
  assert.deepEqual(findInstrumentNode(tree, "MS:3").instruments, [modelA]);
  assert.deepEqual(new Set(findInstrumentNode(tree, "MS:1000463").instruments), new Set([modelA, modelB]));
});

test("plain model names match exactly, and unknown models remain selectable", () => {
  const tree = buildInstrumentTree(terms, ["model a", "New model", "NT=Unrecognized;AC=MS:999"]);
  assert.deepEqual(findInstrumentNode(tree, "MS:3").instruments, ["model a"]);
  assert.deepEqual(findInstrumentNode(tree, "raw:New model").instruments, ["New model"]);
  assert.deepEqual(findInstrumentNode(tree, "raw:NT=Unrecognized;AC=MS:999").instruments, ["NT=Unrecognized;AC=MS:999"]);
});

test("classes with no available datasets do not accidentally match all instruments", () => {
  const tree = buildInstrumentTree(terms, [modelB]);
  assert.deepEqual(findInstrumentNode(tree, "MS:2").instruments, []);
  assert.equal(findInstrumentNode(tree, "missing"), undefined);
});

test("cycles do not recurse forever or duplicate instruments", () => {
  const tree = buildInstrumentTree([
    ...terms,
    { id: "MS:5", name: "Cycle entry", parents: ["MS:1", "MS:6"] },
    { id: "MS:6", name: "Cycle return", parents: ["MS:5"] },
  ], [modelA]);
  assert.deepEqual(findInstrumentNode(tree, "MS:1000463").instruments, [modelA]);
});

test("a selection made before hierarchy loading remains valid", () => {
  const tree = buildInstrumentTree(terms, [modelA]);
  assert.deepEqual(selectedInstruments(tree, `raw:${modelA}`), [modelA]);
  assert.equal(selectedInstruments(tree, ""), null);
  assert.deepEqual(selectedInstruments(tree, "MS:2"), [modelA]);
  assert.deepEqual(selectedInstruments(tree, "missing"), []);
});

test("exact synonyms resolve names without guessing ambiguous matches", () => {
  const tree = buildInstrumentTree([
    { id: "MS:1000463", name: "instrument", parents: [] },
    { id: "MS:1", name: "Model A", parents: ["MS:1000463"], synonyms: ["A", "ambiguous"] },
    { id: "MS:2", name: "Model B", parents: ["MS:1000463"], synonyms: ["ambiguous"] },
  ], ["A", "ambiguous"]);
  assert.deepEqual(findInstrumentNode(tree, "MS:1").instruments, ["A"]);
  assert.deepEqual(findInstrumentNode(tree, "raw:ambiguous").instruments, ["ambiguous"]);
});

test("method filters handle mixed, missing and case-normalized types", () => {
  assert.deepEqual(methodTypes([]), ["Unknown"]);
  assert.equal(matchesMethod(["DDA", "DIA"], "dia"), true);
  assert.equal(matchesMethod(["TMT", "SILAC"], "SILAC"), true);
  assert.equal(matchesMethod([], "Label-free"), false);
  assert.equal(matchesMethod(undefined, "Unknown"), true);
  assert.equal(matchesMethod(["DDA"], ""), true);
});

const info = { quantificationTypes: ["TMT"], acquisitionTypes: ["DDA"], quantificationLabels: ["TMT126"] };

test("enrichment deduplicates versions and updates the UI in bounded batches", async () => {
  const datasets = Array.from({ length: 25 }, (_, i) => ({ accession: `PXD${i}`, datasetRef: `PXD${i}/version` }));
  datasets.push({ ...datasets[0] });
  const batches = [];
  const updates = [];
  await enrichPortalMethods(datasets, async (refs) => {
    batches.push(refs);
    return Object.fromEntries(refs.map((ref) => [ref, info]));
  }, (metadata) => updates.push(metadata), () => true);
  assert.deepEqual(batches.map((batch) => batch.length), [12, 12, 1]);
  assert.equal(updates.length, 3);
  assert.equal(batches.flat().length, 25);
  assert.ok(datasets.every((ds) => ds.quantificationTypes[0] === "TMT"));
});

test("leaving the source stops pending enrichment without modifying its datasets", async () => {
  const datasets = Array.from({ length: 25 }, (_, i) => ({ accession: `PXD${i}`, datasetRef: `PXD${i}/version` }));
  let active = true;
  let calls = 0;
  await enrichPortalMethods(datasets, async (refs) => {
    calls++;
    active = false;
    return Object.fromEntries(refs.map((ref) => [ref, info]));
  }, () => assert.fail("must not publish after unmount"), () => active);
  assert.equal(calls, 1);
  assert.ok(datasets.every((ds) => ds.quantificationTypes === undefined));
});

test("incomplete metadata responses fail explicitly rather than showing unknown success", async () => {
  const datasets = [{ accession: "PXD1", datasetRef: "PXD1/version" }];
  await assert.rejects(enrichPortalMethods(datasets, async () => ({}), () => {}, () => true), /missing dataset/);
  await assert.rejects(enrichPortalMethods(datasets, async () => ({ "PXD1/version": {} }), () => {}, () => true), /invalid types/);
});
