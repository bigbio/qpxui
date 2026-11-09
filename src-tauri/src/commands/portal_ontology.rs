use super::portal_http;
use std::collections::{BTreeMap, BTreeSet};
use tokio::sync::OnceCell;

const INSTRUMENT: &str = "MS:1000463";
const MODEL: &str = "MS:1000031";
const CLASS: &str = "MS:1003761";
const ONTOLOGY_URL: &str = "https://raw.githubusercontent.com/HUPO-PSI/psi-ms-CV/d0f4eca3ed40e220858ea2c4d7c03215b38e48ef/psi-ms.obo";
static TERMS: OnceCell<Vec<InstrumentTerm>> = OnceCell::const_new();

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstrumentTerm {
    id: String,
    name: String,
    parents: Vec<String>,
    synonyms: Vec<String>,
}

#[derive(Default)]
struct Term {
    id: String,
    name: String,
    parents: Vec<String>,
    synonyms: Vec<String>,
    part_of_instrument: bool,
    obsolete: bool,
}

fn instrument_terms(obo: &str) -> Result<Vec<InstrumentTerm>, String> {
    let mut terms = BTreeMap::new();
    let mut current: Option<Term> = None;
    for line in obo.lines().chain(std::iter::once("[end]")) {
        if line.starts_with('[') {
            if let Some(term) = current.take() {
                if !term.obsolete && !term.id.is_empty() && !term.name.is_empty() {
                    terms.insert(term.id.clone(), term);
                }
            }
            if line == "[Term]" {
                current = Some(Term::default());
            }
            continue;
        }
        let Some(term) = current.as_mut() else {
            continue;
        };
        if let Some(id) = line.strip_prefix("id: ") {
            term.id = id.to_string();
        } else if let Some(name) = line.strip_prefix("name: ") {
            term.name = name.to_string();
        } else if let Some(parent) = line.strip_prefix("is_a: ") {
            if let Some(id) = parent.split_whitespace().next() {
                term.parents.push(id.to_string());
            }
        } else if let Some(relationship) = line.strip_prefix("relationship: ") {
            let mut parts = relationship.split_whitespace();
            if parts.next() == Some("part_of") && parts.next() == Some(INSTRUMENT) {
                term.part_of_instrument = true;
            }
        } else if let Some(synonym) = line.strip_prefix("synonym: \"") {
            if let Some((name, rest)) = synonym.split_once('"') {
                if rest.trim_start().starts_with("EXACT") {
                    term.synonyms.push(name.to_string());
                }
            }
        } else if line == "is_obsolete: true" {
            term.obsolete = true;
        }
    }
    if !terms.contains_key(INSTRUMENT) {
        return Err("PSI-MS ontology is missing the instrument root".into());
    }
    for root in [MODEL, CLASS] {
        if !terms
            .get(root)
            .map(|t| t.part_of_instrument)
            .unwrap_or(false)
        {
            return Err(format!(
                "PSI-MS ontology is missing {root}'s part_of instrument relationship"
            ));
        }
    }

    // Only the model and architecture branches belong in this picker, not the
    // instrument's other part_of branches (sources, detectors, and attributes).
    let mut included = BTreeSet::from([MODEL.to_string(), CLASS.to_string()]);
    loop {
        let descendants: Vec<String> = terms
            .values()
            .filter(|t| !included.contains(&t.id) && t.parents.iter().any(|p| included.contains(p)))
            .map(|t| t.id.clone())
            .collect();
        if descendants.is_empty() {
            break;
        }
        included.extend(descendants);
    }
    included.insert(INSTRUMENT.to_string());
    Ok(included
        .iter()
        .map(|id| {
            let term = &terms[id];
            let parents = if id == MODEL || id == CLASS {
                vec![INSTRUMENT.to_string()]
            } else {
                term.parents
                    .iter()
                    .filter(|id| included.contains(*id))
                    .cloned()
                    .collect()
            };
            InstrumentTerm {
                id: id.clone(),
                name: term.name.clone(),
                parents,
                synonyms: term.synonyms.clone(),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn portal_instrument_terms() -> Result<Vec<InstrumentTerm>, String> {
    let terms = TERMS
        .get_or_try_init(|| async {
            let client = portal_http::client_builder()?
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| format!("ontology HTTP client: {e:?}"))?;
            let obo = client
                .get(ONTOLOGY_URL)
                .send()
                .await
                .map_err(|e| format!("fetch PSI-MS ontology: {e:?}"))?
                .error_for_status()
                .map_err(|e| format!("fetch PSI-MS ontology: {e}"))?
                .text()
                .await
                .map_err(|e| format!("read PSI-MS ontology: {e}"))?;
            instrument_terms(&obo)
        })
        .await?;
    Ok(terms.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBO: &str = r#"[Term]
id: MS:1000463
name: instrument

[Term]
id: MS:1000031
name: instrument model
relationship: part_of MS:1000463 ! instrument

[Term]
id: MS:1003761
name: instrument class
relationship: part_of MS:1000463 ! instrument

[Term]
id: MS:1003769
name: quadrupole orbitrap instrument
is_a: MS:1003761 ! instrument class

[Term]
id: MS:1000494
name: vendor instruments
is_a: MS:1000031 ! instrument model

[Term]
id: MS:1001911
name: Q Exactive
synonym: "QE" EXACT []
is_a: MS:1003769 ! quadrupole orbitrap instrument
is_a: MS:1000494 ! vendor instruments

[Term]
id: MS:9
name: old model
is_a: MS:1000494
is_obsolete: true

[Term]
id: MS:10
name: detector
relationship: part_of MS:1000463 ! instrument

[Typedef]
id: part_of
name: part of
"#;

    #[test]
    fn keeps_classes_models_multiple_parents_and_exact_synonyms() {
        let terms = instrument_terms(OBO).unwrap();
        let qe = terms.iter().find(|t| t.id == "MS:1001911").unwrap();
        assert_eq!(qe.parents, ["MS:1003769", "MS:1000494"]);
        assert_eq!(qe.synonyms, ["QE"]);
        for root in [MODEL, CLASS] {
            assert_eq!(
                terms.iter().find(|t| t.id == root).unwrap().parents,
                [INSTRUMENT]
            );
        }
        assert!(!terms.iter().any(|t| t.id == "MS:9" || t.id == "MS:10"));
    }

    #[test]
    fn rejects_invalid_ontology_instead_of_returning_an_empty_tree() {
        assert!(instrument_terms("<html>error</html>").is_err());
        assert!(instrument_terms(&OBO.replace(
            "relationship: part_of MS:1000463",
            "relationship: part_of MS:0"
        ))
        .is_err());
    }

    #[tokio::test]
    #[ignore = "downloads the pinned public PSI-MS ontology"]
    async fn live_psi_ms_includes_architecture_and_vendor_parents() {
        let terms = portal_instrument_terms().await.unwrap();
        assert!(terms.len() > 500);
        let qe = terms.iter().find(|t| t.id == "MS:1001911").unwrap();
        assert_eq!(qe.name, "Q Exactive");
        assert!(qe.parents.contains(&"MS:1003769".to_string()));
        assert!(qe.parents.contains(&"MS:1000494".to_string()));
        let lumos = terms.iter().find(|t| t.id == "MS:1002732").unwrap();
        assert!(lumos.parents.contains(&"MS:1003770".to_string()));
    }
}
