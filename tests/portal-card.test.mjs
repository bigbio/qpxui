import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL } from "node:url";
import { test } from "vitest";
import { compile } from "@vue/compiler-dom";
import * as Vue from "vue";
import { renderToString } from "@vue/server-renderer";

const source = readFileSync(new URL("../src/components/DiscoverView.vue", import.meta.url), "utf8");
const template = source.match(/<div class="links">[\s\S]*?<\/div>/)?.[0];
assert.ok(template, "Dataset card links template must exist");
const { code } = compile(template, { mode: "function", prefixIdentifiers: true });
const render = new Function("Vue", code)(Vue);

async function cardLinks(ds) {
  return renderToString(Vue.createSSRApp({
    setup: () => ({ ds, doiUrl: (doi) => `https://doi.org/${doi}` }),
    render,
  }));
}

test("PRIDE cards show the repository once when a PRIDE link exists", async () => {
  for (const source of ["PRIDE", "pride", " PRIDE "]) {
    const html = await cardLinks({ pxUrl: "https://www.ebi.ac.uk/pride/archive/projects/PXD008840", source });
    assert.equal((html.replace(/<[^>]*>/g, "").match(/PRIDE/gi) ?? []).length, 1);
    assert.ok(!html.includes('<span class="muted">'));
    assert.ok(html.includes('target="_blank"'));
  }
});

test("each available repository URL uses its source name without a duplicate tag", async () => {
  for (const source of ["iProX", "PDC", "MassIVE"]) {
    const url = "https://example.org/project";
    const html = await cardLinks({ source, pxUrl: url });
    assert.ok(html.includes(`href="${url}"`));
    assert.equal(html.replace(/<[^>]*>/g, ""), `${source} ↗`);
    assert.ok(!html.includes("<span"));
  }
});

test("no repository tag or invented link is shown when no URL is available", async () => {
  for (const source of ["PRIDE", "iProX", "PDC", "MassIVE", "unknown"]) {
    const html = await cardLinks({ source });
    assert.equal(html.replace(/<[^>]*>/g, ""), "");
    assert.ok(!html.includes("<a "));
  }
});

test("unidentified repositories get a generic source link while DOI remains independent", async () => {
  for (const source of [null, "", "unknown", " UNKNOWN "]) {
    const html = await cardLinks({ source, pxUrl: "https://example.org/project", publication: { doi: "10.1234/example" } });
    assert.equal(html.replace(/<[^>]*>/g, "").replace(/\s+/g, " ").trim(), "Source ↗ DOI ↗");
    assert.ok(html.includes('href="https://doi.org/10.1234/example"'));
  }
});
