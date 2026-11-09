export function ontologyName(value: string | null | undefined): string {
  if (!value) return "";
  const match = /(?:^|;)NT=([^;]+)/.exec(value);
  return match ? match[1].trim() : value;
}

export interface InstrumentTerm {
  id: string;
  name: string;
  parents: string[];
  synonyms?: string[];
}

export interface InstrumentNode {
  id: string;
  name: string;
  instruments: string[];
  children: InstrumentNode[];
}

export function ontologyAccession(value: string): string | null {
  return /(?:^|;)AC=(MS:\d+)(?:;|$)/.exec(value)?.[1] ?? null;
}

export function buildInstrumentTree(
  terms: InstrumentTerm[],
  instruments: string[],
): InstrumentNode[] {
  const byId = new Map(terms.map((term) => [term.id, term]));
  const children = new Map<string, InstrumentTerm[]>();
  const direct = new Map<string, string[]>();
  const unmatched: InstrumentNode[] = [];
  for (const value of instruments) {
    const accession = ontologyAccession(value);
    const name = ontologyName(value).toLowerCase();
    const named = terms.filter((t) => t.name.toLowerCase() === name || t.synonyms?.some((s) => s.toLowerCase() === name));
    const term = accession ? byId.get(accession) : named.length === 1 ? named[0] : undefined;
    if (term) {
      const values = direct.get(term.id) ?? [];
      values.push(value);
      direct.set(term.id, values);
    } else {
      unmatched.push({ id: `raw:${value}`, name: ontologyName(value), instruments: [value], children: [] });
    }
  }
  for (const term of terms) {
    for (const parent of term.parents) {
      const list = children.get(parent) ?? [];
      list.push(term);
      children.set(parent, list);
    }
  }
  const build = (term: InstrumentTerm, ancestors: Set<string>): InstrumentNode => {
    const path = new Set(ancestors).add(term.id);
    const nodes = (children.get(term.id) ?? [])
      .filter((child) => !path.has(child.id))
      .map((child) => build(child, path))
      .sort((a, b) => a.name.localeCompare(b.name));
    return {
      id: term.id,
      name: term.name,
      instruments: [...new Set([...(direct.get(term.id) ?? []), ...nodes.flatMap((n) => n.instruments)])],
      children: nodes,
    };
  };
  const roots = terms
    .filter((term) => !term.parents.some((parent) => byId.has(parent)))
    .map((term) => build(term, new Set()));
  if (unmatched.length) {
    roots.push({
      id: "unclassified",
      name: "Other portal instruments",
      instruments: unmatched.flatMap((node) => node.instruments),
      children: unmatched.sort((a, b) => a.name.localeCompare(b.name)),
    });
  }
  return roots;
}

export function findInstrumentNode(nodes: InstrumentNode[], id: string): InstrumentNode | undefined {
  for (const node of nodes) {
    if (node.id === id) return node;
    const child = findInstrumentNode(node.children, id);
    if (child) return child;
  }
}

export function selectedInstruments(nodes: InstrumentNode[], id: string): string[] | null {
  if (!id) return null;
  if (id.startsWith("raw:")) return [id.slice(4)];
  return findInstrumentNode(nodes, id)?.instruments ?? [];
}
