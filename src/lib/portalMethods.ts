import type { PortalDataset, PortalMethodInfo } from "./api";

export function methodTypes(types: string[] | null | undefined): string[] {
  return types?.length ? types : ["Unknown"];
}

export function matchesMethod(types: string[] | null | undefined, selected: string): boolean {
  return !selected || methodTypes(types).some((type) => type.toLowerCase() === selected.toLowerCase());
}

export async function enrichPortalMethods(
  datasets: PortalDataset[],
  fetch: (refs: string[]) => Promise<Record<string, PortalMethodInfo>>,
  onBatch: (metadata: Record<string, PortalMethodInfo>) => void,
  active: () => boolean,
): Promise<void> {
  if (!active()) return;
  const refs = [...new Set(datasets.flatMap((ds) => ds.datasetRef ? [ds.datasetRef] : []))];
  for (const dataset of datasets) {
    if (!dataset.datasetRef) dataset.metadataError = "Dataset version reference is unavailable";
  }
  for (let start = 0; start < refs.length && active(); start += 12) {
    const batch = refs.slice(start, start + 12);
    const metadata = await fetch(batch);
    if (!active()) return;
    for (const ref of batch) {
      if (!metadata[ref]) throw new Error(`Method metadata response is missing dataset ${ref}`);
      if (!Array.isArray(metadata[ref].quantificationTypes) || !Array.isArray(metadata[ref].acquisitionTypes)) {
        throw new Error(`Method metadata response has invalid types for dataset ${ref}`);
      }
    }
    for (const dataset of datasets) {
      if (dataset.datasetRef && metadata[dataset.datasetRef]) Object.assign(dataset, metadata[dataset.datasetRef]);
    }
    onBatch(metadata);
  }
}
