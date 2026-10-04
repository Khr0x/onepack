//! Compara la implementación con el corpus generado por `NuGet.Versioning` (ADR-008).
//! Regenerar: `scripts/version-corpus.sh`.

use onepack_nuget::NuGetVersion;
use serde_json::Value;

const CORPUS: &str = include_str!("../../../tests/conformance-dotnet/version-corpus/corpus.json");

struct Expected {
    input: String,
    version: NuGetVersion,
    rank: u64,
    identity: u64,
}

#[test]
fn matches_nuget_versioning() {
    let corpus: Value = serde_json::from_str(CORPUS).unwrap();
    let mut failures = Vec::new();
    let mut valid = Vec::new();

    for case in corpus["cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let parsed = NuGetVersion::parse(input);

        if !case["valid"].as_bool().unwrap() {
            if let Ok(v) = parsed {
                failures.push(format!(
                    "{input:?}: NuGet la rechaza, onepack la acepta como {v}"
                ));
            }
            continue;
        }
        let v = match parsed {
            Ok(v) => v,
            Err(_) => {
                failures.push(format!("{input:?}: NuGet la acepta, onepack la rechaza"));
                continue;
            }
        };
        for (field, actual) in [
            ("normalized", Value::from(v.normalized())),
            ("full", Value::from(v.full())),
            ("prerelease", Value::from(v.is_prerelease())),
            ("semver2", Value::from(v.is_semver2())),
        ] {
            if case[field] != actual {
                failures.push(format!(
                    "{input:?}: {field} esperado {}, obtenido {actual}",
                    case[field]
                ));
            }
        }
        valid.push(Expected {
            input: input.to_owned(),
            version: v,
            rank: case["rank"].as_u64().unwrap(),
            identity: case["identity"].as_u64().unwrap(),
        });
    }

    for a in &valid {
        for b in &valid {
            let expected = a.rank.cmp(&b.rank);
            let actual = a.version.precedence_cmp(&b.version);
            if expected != actual {
                failures.push(format!(
                    "cmp({:?}, {:?}): esperado {expected:?}, obtenido {actual:?}",
                    a.input, b.input
                ));
            }
            let same_expected = a.identity == b.identity;
            let same_actual = a.version.identity() == b.version.identity();
            if same_expected != same_actual {
                failures.push(format!(
                    "identidad({:?}, {:?}): esperado {same_expected}, obtenido {same_actual}",
                    a.input, b.input
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} divergencias:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
