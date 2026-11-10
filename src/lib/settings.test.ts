import { expect, it } from "vitest";
import { DOWNLOAD_CONNECTIONS, effectivePtmColor, resolvePtmColor, setDownloadConnections, settings } from "./settings";

it("uses the longest matching PTM keyword, with accession taking priority", () => {
  expect(resolvePtmColor("Dimethyl (K)")).toBe(effectivePtmColor("dimethyl"));
  expect(resolvePtmColor("Trimethyl (K)")).toBe(effectivePtmColor("trimethyl"));
  expect(resolvePtmColor("Dethiomethyl")).toBe(effectivePtmColor("dimethyl"));
  expect(resolvePtmColor("Methylation")).toBe(effectivePtmColor("methyl"));
  expect(resolvePtmColor("Trimethyl (K)", "UNIMOD:34")).toBe(effectivePtmColor("methyl"));
  expect(resolvePtmColor("Xyzzy")).toBe(effectivePtmColor("unknown"));
});

it("clamps the download connections setting", () => {
  expect(settings.downloadConnections).toBe(DOWNLOAD_CONNECTIONS.default);
  setDownloadConnections(0);
  expect(settings.downloadConnections).toBe(1);
  setDownloadConnections(100);
  expect(settings.downloadConnections).toBe(16);
  setDownloadConnections(4.4);
  expect(settings.downloadConnections).toBe(4);
  setDownloadConnections(Number.NaN);
  expect(settings.downloadConnections).toBe(DOWNLOAD_CONNECTIONS.default);
});
