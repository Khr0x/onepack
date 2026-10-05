//! Documentos del protocolo NuGet V3: registros (`RegistrationsBaseUrl/3.6.0`), búsqueda
//! (`SearchQueryService`) y autocompletado (`SearchAutocompleteService`). Funciones puras: el
//! servidor resuelve permisos y datos, aquí solo se da forma al protocolo (ADR-009).

use std::cmp::Ordering;
use std::collections::BTreeMap;

use onepack_core::PublishedVersion;
use serde_json::{Map, Value, json};

use crate::{NuGetVersion, PackageManifest};

/// Versiones por página de registro.
pub const PAGE_SIZE: usize = 64;
/// Hasta este número de versiones, las páginas van incluidas en el índice.
pub const INLINE_LIMIT: usize = 128;
pub const MAX_TAKE: usize = 1000;
pub const DEFAULT_TAKE: usize = 20;

/// URLs base del feed, sin barra final.
pub struct FeedUrls {
    pub registration: String,
    pub flat: String,
}

impl FeedUrls {
    pub fn new(feed_base: &str) -> Self {
        Self {
            registration: format!("{feed_base}/v3/registration"),
            flat: format!("{feed_base}/v3/flat"),
        }
    }

    pub fn registration_index(&self, package_key: &str) -> String {
        format!("{}/{package_key}/index.json", self.registration)
    }

    fn leaf(&self, v: &PublishedVersion) -> String {
        format!(
            "{}/{}/{}.json",
            self.registration, v.package_key, v.version_key
        )
    }

    fn page(&self, package_key: &str, lower: &str, upper: &str) -> String {
        format!(
            "{}/{package_key}/page/{lower}/{upper}.json",
            self.registration
        )
    }

    fn package_content(&self, v: &PublishedVersion) -> String {
        format!(
            "{}/{id}/{ver}/{id}.{ver}.nupkg",
            self.flat,
            id = v.package_key,
            ver = v.version_key
        )
    }
}

// ---------------------------------------------------------------------------------------------
// Metadatos almacenados
// ---------------------------------------------------------------------------------------------

/// Documento de metadatos que se guarda con cada versión (claves de `catalogEntry`).
pub fn catalog_metadata(m: &PackageManifest) -> Value {
    let md = &m.metadata;
    let mut doc = Map::new();
    let mut put = |key: &str, value: Option<&String>| {
        if let Some(v) = value {
            doc.insert(key.into(), Value::String(v.clone()));
        }
    };
    put("title", md.title.as_ref());
    put("description", md.description.as_ref());
    put("summary", md.summary.as_ref());
    put("authors", md.authors.as_ref());
    put("owners", md.owners.as_ref());
    put("projectUrl", md.project_url.as_ref());
    put("iconUrl", md.icon_url.as_ref());
    put("icon", md.icon.as_ref());
    put("licenseUrl", md.license_url.as_ref());
    put("releaseNotes", md.release_notes.as_ref());
    put("copyright", md.copyright.as_ref());
    put("language", md.language.as_ref());
    put("minClientVersion", md.min_client_version.as_ref());
    put("readme", md.readme.as_ref());
    put("repositoryUrl", md.repository_url.as_ref());
    if let Some(license) = &md.license {
        let key = if license.kind == "file" {
            "licenseFile"
        } else {
            "licenseExpression"
        };
        doc.insert(key.into(), Value::String(license.value.clone()));
    }
    doc.insert("tags".into(), json!(md.tag_list()));
    doc.insert(
        "requireLicenseAcceptance".into(),
        Value::Bool(md.require_license_acceptance),
    );
    doc.insert(
        "packageTypes".into(),
        Value::Array(
            md.package_type_names()
                .into_iter()
                .map(|name| json!({ "name": name }))
                .collect(),
        ),
    );
    doc.insert(
        "dependencyGroups".into(),
        Value::Array(
            m.dependency_groups
                .iter()
                .map(|g| {
                    let mut group = Map::new();
                    if let Some(tf) = &g.target_framework {
                        group.insert("targetFramework".into(), Value::String(tf.clone()));
                    }
                    group.insert(
                        "dependencies".into(),
                        Value::Array(
                            g.dependencies
                                .iter()
                                .map(|d| match &d.range {
                                    Some(range) => json!({ "id": d.id, "range": range }),
                                    None => json!({ "id": d.id }),
                                })
                                .collect(),
                        ),
                    );
                    Value::Object(group)
                })
                .collect(),
        ),
    );
    Value::Object(doc)
}

/// Texto, en minúsculas, sobre el que busca `SearchQueryService`.
pub fn search_text(m: &PackageManifest) -> String {
    let md = &m.metadata;
    [
        Some(m.id.as_str()),
        md.title.as_deref(),
        md.tags.as_deref(),
        md.authors.as_deref(),
        md.summary.as_deref(),
        md.description.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
}

fn metadata(v: &PublishedVersion) -> Map<String, Value> {
    v.metadata
        .as_deref()
        .and_then(|m| serde_json::from_str::<Value>(m).ok())
        .and_then(|m| match m {
            Value::Object(o) => Some(o),
            _ => None,
        })
        .unwrap_or_default()
}

fn str_field<'a>(md: &'a Map<String, Value>, key: &str) -> &'a str {
    md.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list_field(md: &Map<String, Value>, key: &str) -> Vec<String> {
    match md.get(key) {
        Some(Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------------------------
// Orden de versiones
// ---------------------------------------------------------------------------------------------

/// Ordena por precedencia NuGet (ascendente). Las versiones no parseables quedan al principio.
pub fn sort_versions(versions: &mut [PublishedVersion]) {
    versions.sort_by(|a, b| {
        match (
            NuGetVersion::parse(&a.version),
            NuGetVersion::parse(&b.version),
        ) {
            (Ok(x), Ok(y)) => x.precedence_cmp(&y),
            (Err(_), Ok(_)) => Ordering::Less,
            (Ok(_), Err(_)) => Ordering::Greater,
            (Err(_), Err(_)) => a.version_key.cmp(&b.version_key),
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Registros
// ---------------------------------------------------------------------------------------------

fn catalog_entry(urls: &FeedUrls, v: &PublishedVersion) -> Value {
    let md = metadata(v);
    let mut entry = Map::new();
    entry.insert(
        "@id".into(),
        Value::String(format!("{}#catalog", urls.leaf(v))),
    );
    entry.insert("@type".into(), Value::String("PackageDetails".into()));
    entry.insert("id".into(), Value::String(v.package_id.clone()));
    entry.insert("version".into(), Value::String(v.full_version.clone()));
    for key in [
        "title",
        "description",
        "summary",
        "projectUrl",
        "iconUrl",
        "licenseUrl",
        "licenseExpression",
        "language",
        "minClientVersion",
    ] {
        if let Some(value) = md.get(key) {
            entry.insert(key.into(), value.clone());
        }
    }
    entry.insert(
        "authors".into(),
        Value::String(list_field(&md, "authors").join(", ")),
    );
    entry.insert("tags".into(), json!(list_field(&md, "tags")));
    entry.insert(
        "requireLicenseAcceptance".into(),
        md.get("requireLicenseAcceptance")
            .cloned()
            .unwrap_or(Value::Bool(false)),
    );
    entry.insert("listed".into(), Value::Bool(v.listed));
    // Una versión bloqueada (ADR-013) se anuncia como obsoleta para que el IDE lo muestre
    // antes de que falle la descarga. El motivo no se publica: puede ser sensible.
    if v.blocked {
        entry.insert(
            "deprecation".into(),
            json!({
                "@id": format!("{}#deprecation", urls.leaf(v)),
                "reasons": ["Other"],
                "message": "Versión bloqueada por el registro: no se puede descargar.",
            }),
        );
    }
    entry.insert("published".into(), Value::String(v.published_at.clone()));
    entry.insert(
        "packageContent".into(),
        Value::String(urls.package_content(v)),
    );

    let groups = md
        .get("dependencyGroups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|mut group| {
            if let Some(Value::Array(deps)) = group.get_mut("dependencies") {
                for dep in deps.iter_mut() {
                    let key = dep.get("id").and_then(Value::as_str).map(str::to_lowercase);
                    if let (Some(key), Value::Object(d)) = (key, dep) {
                        d.insert("@type".into(), Value::String("PackageDependency".into()));
                        d.insert(
                            "registration".into(),
                            Value::String(urls.registration_index(&key)),
                        );
                    }
                }
            }
            if let Value::Object(g) = &mut group {
                g.insert(
                    "@type".into(),
                    Value::String("PackageDependencyGroup".into()),
                );
            }
            group
        })
        .collect();
    entry.insert("dependencyGroups".into(), Value::Array(groups));
    Value::Object(entry)
}

fn page_leaf(urls: &FeedUrls, v: &PublishedVersion) -> Value {
    json!({
        "@id": urls.leaf(v),
        "@type": "Package",
        "catalogEntry": catalog_entry(urls, v),
        "packageContent": urls.package_content(v),
        "registration": urls.registration_index(&v.package_key),
    })
}

fn page(urls: &FeedUrls, chunk: &[PublishedVersion], inline: bool) -> Value {
    let first = chunk.first().expect("página no vacía");
    let last = chunk.last().expect("página no vacía");
    let mut page = json!({
        "@id": urls.page(&first.package_key, &first.version_key, &last.version_key),
        "@type": "catalog:CatalogPage",
        "count": chunk.len(),
        "lower": first.version_key,
        "upper": last.version_key,
        "parent": urls.registration_index(&first.package_key),
    });
    if inline {
        page["items"] = Value::Array(chunk.iter().map(|v| page_leaf(urls, v)).collect());
    }
    page
}

/// Índice de registro de un paquete. Incluye versiones no listadas con `listed: false`.
pub fn registration_index(urls: &FeedUrls, versions: &[PublishedVersion]) -> Option<Value> {
    let mut versions = versions.to_vec();
    sort_versions(&mut versions);
    let first = versions.first()?;
    let inline = versions.len() <= INLINE_LIMIT;
    let pages: Vec<Value> = versions
        .chunks(PAGE_SIZE)
        .map(|chunk| page(urls, chunk, inline))
        .collect();
    Some(json!({
        "@id": urls.registration_index(&first.package_key),
        "@type": ["catalog:CatalogRoot", "PackageRegistration", "catalog:Permalink"],
        "count": pages.len(),
        "items": pages,
    }))
}

/// Página de registro identificada por sus límites (`lower`/`upper`, claves de versión).
pub fn registration_page(
    urls: &FeedUrls,
    versions: &[PublishedVersion],
    lower: &str,
    upper: &str,
) -> Option<Value> {
    let mut versions = versions.to_vec();
    sort_versions(&mut versions);
    versions
        .chunks(PAGE_SIZE)
        .find(|c| c[0].version_key == lower && c[c.len() - 1].version_key == upper)
        .map(|chunk| page(urls, chunk, true))
}

/// Hoja de registro de una versión.
pub fn registration_leaf(urls: &FeedUrls, v: &PublishedVersion) -> Value {
    json!({
        "@id": urls.leaf(v),
        "@type": ["Package", "http://schema.nuget.org/catalog#Permalink"],
        "catalogEntry": format!("{}#catalog", urls.leaf(v)),
        "listed": v.listed,
        "packageContent": urls.package_content(v),
        "published": v.published_at,
        "registration": urls.registration_index(&v.package_key),
    })
}

// ---------------------------------------------------------------------------------------------
// Búsqueda y autocompletado
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub q: String,
    pub skip: usize,
    pub take: usize,
    pub package_type: Option<String>,
}

/// Versiones listadas de un feed agrupadas por paquete (claves en orden) y con las versiones
/// ordenadas por precedencia. Construirlo cuesta; reutilizarlo entre búsquedas es barato, así
/// que el servidor lo guarda en caché.
#[derive(Debug, Clone, Default)]
pub struct SearchIndex {
    packages: Vec<(String, Vec<PublishedVersion>)>,
}

impl SearchIndex {
    pub fn new(versions: Vec<PublishedVersion>) -> Self {
        Self {
            packages: by_package(versions).into_iter().collect(),
        }
    }

    /// Versiones de un paquete, en orden ascendente.
    pub fn versions_of(&self, package_key: &str) -> &[PublishedVersion] {
        self.packages
            .binary_search_by(|(key, _)| key.as_str().cmp(package_key))
            .map_or(&[], |i| self.packages[i].1.as_slice())
    }
}

/// Agrupa por paquete (claves en orden) con las versiones ordenadas por precedencia.
fn by_package(versions: Vec<PublishedVersion>) -> BTreeMap<String, Vec<PublishedVersion>> {
    let mut packages: BTreeMap<String, Vec<PublishedVersion>> = BTreeMap::new();
    for v in versions {
        packages.entry(v.package_key.clone()).or_default().push(v);
    }
    for versions in packages.values_mut() {
        sort_versions(versions);
    }
    packages
}

fn has_package_type(latest: &PublishedVersion, wanted: Option<&str>) -> bool {
    let Some(wanted) = wanted.filter(|w| !w.is_empty()) else {
        return true;
    };
    let md = metadata(latest);
    let names: Vec<String> = md
        .get("packageTypes")
        .and_then(Value::as_array)
        .map(|types| {
            types
                .iter()
                .filter_map(|t| t.get("name")?.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_else(|| vec!["Dependency".into()]);
    names.iter().any(|n| n.eq_ignore_ascii_case(wanted))
}

/// Relevancia: id exacto > id que empieza por la consulta > id que contiene un término > resto.
fn score(package_key: &str, q: &str, terms: &[String]) -> u8 {
    if q.is_empty() {
        0
    } else if package_key == q {
        3
    } else if package_key.starts_with(q) {
        2
    } else if terms.iter().any(|t| package_key.contains(t.as_str())) {
        1
    } else {
        0
    }
}

/// Busca entre versiones ya filtradas por listado, prerelease y SemVer 2.0.0.
///
/// Sintaxis: términos libres (todos deben aparecer en id, título, etiquetas, autores, resumen o
/// descripción), `id:texto` (el id contiene) y `packageid:Id` (id exacto).
pub fn search(urls: &FeedUrls, index: &SearchIndex, query: &SearchQuery) -> Value {
    let q = query.q.trim().to_lowercase();
    let mut exact_id = None;
    let mut id_terms = Vec::new();
    let mut terms = Vec::new();
    for token in q.split_whitespace() {
        if let Some(id) = token.strip_prefix("packageid:") {
            exact_id = Some(id.to_owned());
        } else if let Some(id) = token.strip_prefix("id:") {
            id_terms.push(id.to_owned());
        } else {
            terms.push(token.to_owned());
        }
    }

    let mut matches: Vec<(u8, &str, &[PublishedVersion])> = index
        .packages
        .iter()
        .filter(|(key, versions)| {
            let latest = versions.last().expect("al menos una versión");
            let text = latest.search_text.as_deref().unwrap_or(key);
            exact_id.as_ref().is_none_or(|id| id == key)
                && id_terms.iter().all(|t| key.contains(t.as_str()))
                && terms.iter().all(|t| text.contains(t.as_str()))
                && has_package_type(latest, query.package_type.as_deref())
        })
        .map(|(key, versions)| (score(key, &q, &terms), key.as_str(), versions.as_slice()))
        .collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));

    let total = matches.len();
    let take = query.take.min(MAX_TAKE);
    let data: Vec<Value> = matches
        .into_iter()
        .skip(query.skip)
        .take(take)
        .map(|(_, key, versions)| search_result(urls, key, versions))
        .collect();
    json!({ "totalHits": total, "data": data })
}

fn search_result(urls: &FeedUrls, package_key: &str, versions: &[PublishedVersion]) -> Value {
    let latest = versions.last().expect("al menos una versión");
    let md = metadata(latest);
    let registration = urls.registration_index(package_key);
    let types: Vec<Value> = md
        .get("packageTypes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![json!({ "name": "Dependency" })]);
    json!({
        "@id": registration,
        "@type": "Package",
        "registration": registration,
        "id": latest.package_id,
        "version": latest.full_version,
        "description": str_field(&md, "description"),
        "summary": str_field(&md, "summary"),
        "title": str_field(&md, "title"),
        "iconUrl": str_field(&md, "iconUrl"),
        "licenseUrl": str_field(&md, "licenseUrl"),
        "projectUrl": str_field(&md, "projectUrl"),
        "tags": list_field(&md, "tags"),
        "authors": list_field(&md, "authors"),
        "owners": list_field(&md, "owners"),
        "totalDownloads": 0,
        "verified": false,
        "packageTypes": types,
        "versions": versions.iter().map(|v| json!({
            "version": v.full_version,
            "downloads": 0,
            "@id": urls.leaf(v),
        })).collect::<Vec<_>>(),
    })
}

/// Ids de paquete que contienen la consulta; primero los que empiezan por ella.
pub fn autocomplete_ids(index: &SearchIndex, query: &SearchQuery) -> Value {
    let q = query.q.trim().to_lowercase();
    let mut ids: Vec<(bool, &str, &str)> = index
        .packages
        .iter()
        .filter(|(key, versions)| {
            key.contains(q.as_str())
                && has_package_type(
                    versions.last().expect("versión"),
                    query.package_type.as_deref(),
                )
        })
        .map(|(key, versions)| {
            let id = versions.last().expect("versión").package_id.as_str();
            (!key.starts_with(q.as_str()), key.as_str(), id)
        })
        .collect();
    ids.sort();
    let total = ids.len();
    let data: Vec<&str> = ids
        .into_iter()
        .skip(query.skip)
        .take(query.take.min(MAX_TAKE))
        .map(|(_, _, id)| id)
        .collect();
    json!({ "totalHits": total, "data": data })
}

/// Versiones de un id, ya en orden ascendente (`SearchIndex::versions_of`).
pub fn autocomplete_versions(versions: &[PublishedVersion]) -> Value {
    let data: Vec<&str> = versions.iter().map(|v| v.version.as_str()).collect();
    json!({ "totalHits": data.len(), "data": data })
}
