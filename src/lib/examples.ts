// Curated example datasets for the start page, grouped by what they show off.
// Picked from the quantms portal catalogue: small ones open in seconds, the
// large ones are there to show streaming. Datasets are matched by their run
// ref; if the portal reprocessed one (new ref), by accession and organism.
import { portalRef, type PortalDataset } from "./api";

export interface Example {
  /** `<accession>/<run>` on the portal. */
  ref: string;
  /** Why it is worth opening, shown under its title. */
  note: string;
}

export interface ExampleGroup {
  title: string;
  intro: string;
  examples: Example[];
}

export const EXAMPLE_GROUPS: ExampleGroup[] = [
  {
    title: "Quick start",
    intro: "Small datasets (under 500 MB) that stream in seconds.",
    examples: [
      { ref: "PXD004452/eac302ad85f8", note: "Deep HeLa proteome: 9,400 proteins from one cell line" },
      { ref: "PXD036643/045c46fe85c4", note: "Hepatocellular carcinoma cell lines, 9 samples" },
      { ref: "PXD002395/8719514dc06e", note: "11 human cell lines compared, 33 samples" },
    ],
  },
  {
    title: "Many samples",
    intro: "Per-run QC and sample comparisons across a cohort.",
    examples: [
      { ref: "PXD042233/b4c47ddca720", note: "500 HeLa quality-control runs" },
      { ref: "PMID23933261/8b3c17c5e1f3", note: "120 samples of human cell lines" },
    ],
  },
  {
    title: "Beyond human",
    intro: "From The Proteome Landscape of the Kingdoms of Life (PXD014877).",
    examples: [
      { ref: "PXD014877/161d2aebe83d", note: "Mouse" },
      { ref: "PXD014877/db94cf655cd3", note: "Zebrafish" },
      { ref: "PXD014877/b47e2d083369", note: "Bread wheat: 60,000 proteins" },
    ],
  },
  {
    title: "PTMs and immunopeptidomics",
    intro: "Modification sites and HLA ligands.",
    examples: [
      { ref: "PXD014525/1d3f20844724", note: "Site-specific phosphoproteome, 185 samples" },
      { ref: "PXD019643/46da54d6e3e9", note: "HLA Ligand Atlas: natural HLA ligands across tissues" },
    ],
  },
  {
    title: "Large scale",
    intro: "Tens to hundreds of GB: stream these rather than downloading them.",
    examples: [
      { ref: "PXD030304/fb6d543960a7", note: "Pan-cancer map of 949 cell lines" },
      { ref: "HuiYan-E480/392295b50151", note: "58 million PSMs from 763 samples" },
    ],
  },
];

/** Datasets at least this large get a "large" badge and a streaming hint. */
export const LARGE_BYTES = 10e9;

/** How much the catalogue knows about a dataset, to choose among duplicates. */
function richness(d: PortalDataset): number {
  return (d.psmCount ?? 0) + (d.peptides ?? 0) + (d.proteins ?? 0) + (d.samples ?? 0);
}

/** The groups' examples found in the portal registry, in curated order.
 * Examples the portal no longer has are left out, and so are empty groups. */
export function pickExamples(
  registry: PortalDataset[],
  groups: ExampleGroup[] = EXAMPLE_GROUPS
): { group: ExampleGroup; items: { example: Example; dataset: PortalDataset }[] }[] {
  const best = (list: PortalDataset[]) =>
    list.reduce<PortalDataset | undefined>((a, d) => (!a || richness(d) > richness(a) ? d : a), undefined);
  return groups
    .map((group) => ({
      group,
      items: group.examples.flatMap((example) => {
        let dataset = best(registry.filter((d) => portalRef(d) === example.ref));
        if (!dataset) {
          // Reprocessed (new run ref): fall back to the accession, but only
          // when it is unambiguous (one organism, as PXD014877 has many).
          const acc = example.ref.split("/")[0];
          const runs = registry.filter((d) => d.accession === acc);
          if (new Set(runs.map((d) => d.organism)).size === 1) dataset = best(runs);
        }
        return dataset ? [{ example, dataset }] : [];
      }),
    }))
    .filter((g) => g.items.length);
}
