//! Lectura de `.nupkg`: localiza el `.nuspec` en la raíz del ZIP y extrae la identidad, las
//! dependencias y los metadatos. No extrae nada a disco ni ejecuta contenido del paquete
//! (ADR-014).

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::{NuGetVersion, PackageId};

#[derive(Debug, Clone)]
pub struct PackageManifest {
    pub id: PackageId,
    pub version: NuGetVersion,
    pub dependency_groups: Vec<DependencyGroup>,
    pub metadata: Metadata,
    /// Bytes originales del `.nuspec`, para servirlos sin regenerarlos.
    pub nuspec: Vec<u8>,
}

/// Metadatos descriptivos del `.nuspec` que exponen los registros y la búsqueda.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub summary: Option<String>,
    /// Lista separada por comas, tal como aparece en el `.nuspec`.
    pub authors: Option<String>,
    pub owners: Option<String>,
    /// Lista separada por espacios, tal como aparece en el `.nuspec`.
    pub tags: Option<String>,
    pub project_url: Option<String>,
    pub icon_url: Option<String>,
    /// Ruta del icono embebido en el paquete.
    pub icon: Option<String>,
    pub license_url: Option<String>,
    pub license: Option<License>,
    pub require_license_acceptance: bool,
    pub release_notes: Option<String>,
    pub copyright: Option<String>,
    pub language: Option<String>,
    pub min_client_version: Option<String>,
    /// Ruta del readme embebido en el paquete.
    pub readme: Option<String>,
    pub repository_url: Option<String>,
    pub package_types: Vec<PackageType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct License {
    /// `expression` o `file`.
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageType {
    pub name: String,
    pub version: Option<String>,
}

impl Metadata {
    pub fn tag_list(&self) -> Vec<&str> {
        self.tags
            .as_deref()
            .map(|t| t.split([' ', ',', ';']).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    }

    pub fn author_list(&self) -> Vec<&str> {
        split_list(self.authors.as_deref())
    }

    pub fn owner_list(&self) -> Vec<&str> {
        split_list(self.owners.as_deref())
    }

    /// Tipos de paquete; sin declarar, NuGet asume `Dependency`.
    pub fn package_type_names(&self) -> Vec<&str> {
        if self.package_types.is_empty() {
            vec!["Dependency"]
        } else {
            self.package_types.iter().map(|t| t.name.as_str()).collect()
        }
    }
}

fn split_list(value: Option<&str>) -> Vec<&str> {
    value
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyGroup {
    pub target_framework: Option<String>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    pub range: Option<String>,
}

/// Presupuesto de inspección de un paquete no confiable (ADR-014). Los límites son del
/// registro, no de NuGet: un paquete legítimo muy grande puede requerir ampliarlos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionLimits {
    /// Número máximo de entradas del ZIP.
    pub max_entries: u64,
    /// Tamaño descomprimido máximo de una entrada, según el directorio central.
    pub max_entry_bytes: u64,
    /// Suma máxima de los tamaños descomprimidos de todas las entradas.
    pub max_total_bytes: u64,
    /// Tamaño máximo del `.nuspec` descomprimido.
    pub max_nuspec_bytes: u64,
    /// Profundidad máxima de anidamiento de elementos del `.nuspec`.
    pub max_xml_depth: usize,
    /// Tiempo máximo de inspección.
    pub timeout: Duration,
}

impl Default for InspectionLimits {
    fn default() -> Self {
        Self {
            max_entries: 20_000,
            max_entry_bytes: 512 << 20,
            max_total_bytes: 2 << 30,
            max_nuspec_bytes: 1 << 20,
            max_xml_depth: 32,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub enum PackageError {
    NotAZip(zip::result::ZipError),
    NuspecMissing,
    MultipleNuspecs,
    InvalidNuspec(String),
    NuspecTooLarge {
        limit: u64,
    },
    TooManyEntries {
        limit: u64,
    },
    EntryTooLarge {
        name: String,
        limit: u64,
    },
    TooLargeUncompressed {
        limit: u64,
    },
    XmlTooDeep {
        limit: usize,
    },
    Timeout,
    /// Ruta absoluta, con `..`, con caracteres de control o con un nombre reservado.
    UnsafePath(String),
    DuplicateEntry(String),
    UnsupportedEntry {
        name: String,
        reason: &'static str,
    },
}

impl PackageError {
    /// Código estable para clientes y auditoría.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAZip(_)
            | Self::NuspecMissing
            | Self::MultipleNuspecs
            | Self::InvalidNuspec(_)
            | Self::UnsupportedEntry { .. } => "PACKAGE_INVALID",
            Self::NuspecTooLarge { .. }
            | Self::TooManyEntries { .. }
            | Self::EntryTooLarge { .. }
            | Self::TooLargeUncompressed { .. }
            | Self::XmlTooDeep { .. }
            | Self::Timeout => "PACKAGE_LIMIT_EXCEEDED",
            Self::UnsafePath(_) | Self::DuplicateEntry(_) => "PACKAGE_UNSAFE_PATH",
        }
    }
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.code())?;
        match self {
            Self::NotAZip(e) => write!(f, "el paquete no es un ZIP válido: {e}"),
            Self::NuspecMissing => f.write_str("el paquete no contiene un .nuspec en la raíz"),
            Self::MultipleNuspecs => {
                f.write_str("el paquete contiene más de un .nuspec en la raíz")
            }
            Self::InvalidNuspec(msg) => write!(f, ".nuspec inválido: {msg}"),
            Self::NuspecTooLarge { limit } => write!(f, "el .nuspec supera {limit} bytes"),
            Self::TooManyEntries { limit } => {
                write!(f, "el paquete tiene más de {limit} entradas")
            }
            Self::EntryTooLarge { name, limit } => {
                write!(f, "la entrada {name:?} supera {limit} bytes descomprimida")
            }
            Self::TooLargeUncompressed { limit } => {
                write!(f, "el paquete supera {limit} bytes descomprimido")
            }
            Self::XmlTooDeep { limit } => {
                write!(f, "el .nuspec supera {limit} niveles de anidamiento")
            }
            Self::Timeout => f.write_str("la inspección del paquete superó el tiempo máximo"),
            Self::UnsafePath(name) => write!(f, "ruta no permitida en el paquete: {name:?}"),
            Self::DuplicateEntry(name) => write!(f, "entrada duplicada en el paquete: {name:?}"),
            Self::UnsupportedEntry { name, reason } => {
                write!(f, "entrada no admitida ({reason}): {name:?}")
            }
        }
    }
}

impl std::error::Error for PackageError {}

/// Lee el manifiesto con los límites por defecto.
pub fn read_package(nupkg: &[u8]) -> Result<PackageManifest, PackageError> {
    read_package_from(Cursor::new(nupkg), &InspectionLimits::default())
}

/// Inspecciona un paquete sin cargarlo completo en memoria (p. ej. desde un archivo): valida
/// todas las entradas del ZIP y lee el manifiesto.
pub fn read_package_from<R: Read + Seek>(
    nupkg: R,
    limits: &InspectionLimits,
) -> Result<PackageManifest, PackageError> {
    let deadline = Deadline::new(limits.timeout);
    parse_nuspec_within(
        read_nuspec_within(nupkg, limits, &deadline)?,
        limits,
        &deadline,
    )
}

/// Devuelve solo los bytes del `.nuspec` (para servir `/{id}/{version}/{id}.nuspec`).
pub fn read_nuspec_from<R: Read + Seek>(
    nupkg: R,
    limits: &InspectionLimits,
) -> Result<Vec<u8>, PackageError> {
    read_nuspec_within(nupkg, limits, &Deadline::new(limits.timeout))
}

pub fn parse_nuspec(
    nuspec: Vec<u8>,
    limits: &InspectionLimits,
) -> Result<PackageManifest, PackageError> {
    parse_nuspec_within(nuspec, limits, &Deadline::new(limits.timeout))
}

struct Deadline(Instant);

impl Deadline {
    fn new(timeout: Duration) -> Self {
        Self(Instant::now() + timeout)
    }

    fn check(&self) -> Result<(), PackageError> {
        if Instant::now() >= self.0 {
            Err(PackageError::Timeout)
        } else {
            Ok(())
        }
    }
}

fn read_nuspec_within<R: Read + Seek>(
    mut nupkg: R,
    limits: &InspectionLimits,
    deadline: &Deadline,
) -> Result<Vec<u8>, PackageError> {
    // El número de entradas se comprueba antes de que el lector del ZIP reserve memoria
    // para todo el directorio central.
    if declared_entries(&mut nupkg)
        .map_err(|e| PackageError::NotAZip(e.into()))?
        .is_some_and(|n| n > limits.max_entries)
    {
        return Err(PackageError::TooManyEntries {
            limit: limits.max_entries,
        });
    }
    let mut archive = zip::ZipArchive::new(nupkg).map_err(PackageError::NotAZip)?;
    if archive.len() as u64 > limits.max_entries {
        return Err(PackageError::TooManyEntries {
            limit: limits.max_entries,
        });
    }

    let mut seen = HashSet::new();
    let mut total: u64 = 0;
    let mut found = None;
    for i in 0..archive.len() {
        deadline.check()?;
        // Solo el directorio central: nada se descomprime ni se escribe a disco.
        let entry = archive.by_index_raw(i).map_err(PackageError::NotAZip)?;
        let name = entry.name().to_owned();
        check_entry_name(&name)?;
        if !seen.insert(canonical_name(&name)) {
            return Err(PackageError::DuplicateEntry(name));
        }
        if entry.is_symlink() {
            return Err(PackageError::UnsupportedEntry {
                name,
                reason: "enlace simbólico",
            });
        }
        if entry.encrypted() {
            return Err(PackageError::UnsupportedEntry {
                name,
                reason: "cifrada",
            });
        }
        if entry.size() > limits.max_entry_bytes {
            return Err(PackageError::EntryTooLarge {
                name,
                limit: limits.max_entry_bytes,
            });
        }
        total = total.saturating_add(entry.size());
        if total > limits.max_total_bytes {
            return Err(PackageError::TooLargeUncompressed {
                limit: limits.max_total_bytes,
            });
        }
        if !name.contains('/') && name.to_ascii_lowercase().ends_with(".nuspec") {
            if found.is_some() {
                return Err(PackageError::MultipleNuspecs);
            }
            found = Some(i);
        }
    }
    let index = found.ok_or(PackageError::NuspecMissing)?;

    // El tamaño declarado puede mentir: la lectura se corta en el límite pase lo que pase.
    let entry = archive.by_index(index).map_err(PackageError::NotAZip)?;
    let mut bytes = Vec::new();
    entry
        .take(limits.max_nuspec_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PackageError::InvalidNuspec(e.to_string()))?;
    if bytes.len() as u64 > limits.max_nuspec_bytes {
        return Err(PackageError::NuspecTooLarge {
            limit: limits.max_nuspec_bytes,
        });
    }
    deadline.check()?;
    Ok(bytes)
}

/// Número de entradas que declara el registro de fin de directorio central (EOCD, o EOCD64
/// en ZIP64). `None` si no se encuentra: el lector del ZIP dará el error.
fn declared_entries<R: Read + Seek>(r: &mut R) -> std::io::Result<Option<u64>> {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const EOCD_LEN: u64 = 22;
    let len = r.seek(SeekFrom::End(0))?;
    // El EOCD va al final, seguido de un comentario de hasta 65535 bytes.
    let window = len.min(EOCD_LEN + u64::from(u16::MAX));
    r.seek(SeekFrom::Start(len - window))?;
    let mut tail = vec![0; window as usize];
    r.read_exact(&mut tail)?;
    r.rewind()?;

    let Some(pos) = (0..tail.len().saturating_sub(EOCD_LEN as usize - 1))
        .rev()
        .find(|&p| tail[p..p + 4] == EOCD)
    else {
        return Ok(None);
    };
    let entries = u16::from_le_bytes([tail[pos + 10], tail[pos + 11]]);
    if entries != u16::MAX {
        return Ok(Some(u64::from(entries)));
    }

    // ZIP64: el localizador (20 bytes) precede al EOCD y apunta al EOCD64.
    const LOCATOR: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
    const EOCD64: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];
    if pos < 20 || tail[pos - 20..pos - 16] != LOCATOR {
        return Ok(None);
    }
    let offset = u64::from_le_bytes(tail[pos - 12..pos - 4].try_into().expect("8 bytes"));
    if offset.saturating_add(40) > len {
        return Ok(None);
    }
    let mut record = [0; 40];
    r.seek(SeekFrom::Start(offset))?;
    r.read_exact(&mut record)?;
    r.rewind()?;
    if record[..4] != EOCD64 {
        return Ok(None);
    }
    Ok(Some(u64::from_le_bytes(
        record[32..40].try_into().expect("8 bytes"),
    )))
}

/// Rechaza rutas que, al extraerse en un cliente, escaparían del directorio del paquete o
/// fallarían en Windows. Se comprueba el nombre tal cual y con el escape `%XX` de OPC
/// resuelto, porque los clientes NuGet lo deshacen al extraer.
fn check_entry_name(name: &str) -> Result<(), PackageError> {
    let unsafe_path = || PackageError::UnsafePath(name.to_owned());
    let decoded = percent_decode(name).ok_or_else(unsafe_path)?;
    for candidate in [name, decoded.as_str()] {
        if !is_safe_path(candidate) {
            return Err(unsafe_path());
        }
    }
    Ok(())
}

fn is_safe_path(name: &str) -> bool {
    if name.is_empty() || name.chars().any(|c| c.is_control() || c == ':') {
        return false;
    }
    let path = name.replace('\\', "/");
    if path.starts_with('/') {
        return false;
    }
    // El último segmento vacío es el de una entrada de directorio (`lib/`).
    let segments: Vec<&str> = path.split('/').collect();
    let (last, rest) = segments
        .split_last()
        .expect("split devuelve al menos un segmento");
    rest.iter().all(|s| is_safe_segment(s)) && (last.is_empty() || is_safe_segment(last))
}

fn is_safe_segment(segment: &str) -> bool {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    if segment.is_empty() || segment == "." || segment == ".." {
        return false;
    }
    // Windows ignora los puntos y espacios finales: `a.` y `a ` serían `a`.
    if segment.ends_with(['.', ' ']) {
        return false;
    }
    let stem = segment.split('.').next().unwrap_or_default().trim_end();
    let upper = stem.to_ascii_uppercase();
    let numbered_device = (upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.len() == 4
        && upper.as_bytes()[3].is_ascii_digit();
    !(RESERVED.contains(&upper.as_str()) || numbered_device)
}

/// Deshace `%XX`. `None` si el resultado no es UTF-8 o hay un escape incompleto.
fn percent_decode(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Forma con la que se detectan duplicados: como los vería un sistema de archivos que no
/// distingue mayúsculas, tras deshacer el escape.
fn canonical_name(name: &str) -> String {
    percent_decode(name)
        .unwrap_or_else(|| name.to_owned())
        .replace('\\', "/")
        .to_lowercase()
}

fn parse_nuspec_within(
    nuspec: Vec<u8>,
    limits: &InspectionLimits,
    deadline: &Deadline,
) -> Result<PackageManifest, PackageError> {
    let invalid = |msg: String| PackageError::InvalidNuspec(msg);
    let mut reader = Reader::from_reader(nuspec.as_slice());
    // Sin recortar: el texto llega en trozos separados por las entidades (`A &amp; B`) y
    // recortarlos se comería los espacios. Se recorta el valor final.
    reader.config_mut().trim_text(false);

    let mut path: Vec<String> = Vec::new();
    let mut texts: HashMap<String, String> = HashMap::new();
    let mut groups: Vec<DependencyGroup> = Vec::new();
    let mut metadata = Metadata::default();
    let mut buf = Vec::new();

    loop {
        deadline.check()?;
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| invalid(e.to_string()))?;
        match event {
            // Sin DTD no hay entidades definidas por el documento ni externas: descarta las
            // XML bombs (billion laughs) y XXE.
            Event::DocType(_) => return Err(invalid("DTD no permitido".into())),
            Event::Start(e) => {
                if path.len() >= limits.max_xml_depth {
                    return Err(PackageError::XmlTooDeep {
                        limit: limits.max_xml_depth,
                    });
                }
                on_element(&e, &path, &mut groups, &mut metadata)?;
                path.push(local_name(&e));
            }
            Event::Empty(e) => on_element(&e, &path, &mut groups, &mut metadata)?,
            Event::End(_) => {
                path.pop();
            }
            Event::Text(t) => {
                if let Some(field) = metadata_field(&path) {
                    texts
                        .entry(field.to_owned())
                        .or_default()
                        .push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(field) = metadata_field(&path) {
                    texts
                        .entry(field.to_owned())
                        .or_default()
                        .push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) => {
                let Some(field) = metadata_field(&path) else {
                    continue;
                };
                let resolved = if r.is_char_ref() {
                    r.resolve_char_ref()
                        .map_err(|e| invalid(e.to_string()))?
                        .map(String::from)
                } else {
                    resolve_xml_entity(&r.xml10_content()).map(str::to_owned)
                };
                let resolved =
                    resolved.ok_or_else(|| invalid("entidad XML no permitida".into()))?;
                texts
                    .entry(field.to_owned())
                    .or_default()
                    .push_str(&resolved);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let mut take = |name: &str| {
        texts
            .remove(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let id = take("id").ok_or_else(|| invalid("falta <id>".into()))?;
    let version = take("version").ok_or_else(|| invalid("falta <version>".into()))?;
    metadata.title = take("title");
    metadata.description = take("description");
    metadata.summary = take("summary");
    metadata.authors = take("authors");
    metadata.owners = take("owners");
    metadata.tags = take("tags");
    metadata.project_url = take("projectUrl");
    metadata.icon_url = take("iconUrl");
    metadata.icon = take("icon");
    metadata.license_url = take("licenseUrl");
    metadata.require_license_acceptance =
        take("requireLicenseAcceptance").is_some_and(|v| v.eq_ignore_ascii_case("true"));
    metadata.release_notes = take("releaseNotes");
    metadata.copyright = take("copyright");
    metadata.language = take("language");
    metadata.readme = take("readme");
    if let (Some(license), Some(value)) = (metadata.license.as_mut(), take("license")) {
        license.value = value;
    } else {
        metadata.license = None;
    }

    Ok(PackageManifest {
        id: PackageId::parse(&id).map_err(|e| invalid(e.to_string()))?,
        version: NuGetVersion::parse(&version).map_err(|e| invalid(e.to_string()))?,
        dependency_groups: groups,
        metadata,
        nuspec,
    })
}

/// Campo de texto de `<metadata>` en el que se está dentro, si lo hay.
fn metadata_field(path: &[String]) -> Option<&str> {
    match path {
        [package, metadata, field] if package == "package" && metadata == "metadata" => Some(field),
        _ => None,
    }
}

fn on_element(
    e: &BytesStart<'_>,
    parent: &[String],
    groups: &mut Vec<DependencyGroup>,
    metadata: &mut Metadata,
) -> Result<(), PackageError> {
    let name = local_name(e);
    match (path_str(parent).as_str(), name.as_str()) {
        ("package", "metadata") => metadata.min_client_version = attr(e, "minClientVersion")?,
        ("package/metadata", "license") => {
            metadata.license = Some(License {
                kind: attr(e, "type")?.unwrap_or_else(|| "expression".into()),
                value: String::new(),
            });
        }
        ("package/metadata", "repository") => metadata.repository_url = attr(e, "url")?,
        ("package/metadata/packageTypes", "packageType") => {
            if let Some(name) = attr(e, "name")? {
                metadata.package_types.push(PackageType {
                    name,
                    version: attr(e, "version")?,
                });
            }
        }
        ("package/metadata/dependencies", "group") => groups.push(DependencyGroup {
            target_framework: attr(e, "targetFramework")?,
            dependencies: Vec::new(),
        }),
        // Formato antiguo: dependencias sin grupo equivalen a un grupo sin framework.
        ("package/metadata/dependencies", "dependency") => {
            if groups.first().is_none_or(|g| g.target_framework.is_some()) {
                groups.insert(
                    0,
                    DependencyGroup {
                        target_framework: None,
                        dependencies: Vec::new(),
                    },
                );
            }
            groups[0].dependencies.push(dependency(e)?);
        }
        ("package/metadata/dependencies/group", "dependency") => {
            if let Some(group) = groups.last_mut() {
                group.dependencies.push(dependency(e)?);
            }
        }
        _ => {}
    }
    Ok(())
}

fn dependency(e: &BytesStart<'_>) -> Result<Dependency, PackageError> {
    Ok(Dependency {
        id: attr(e, "id")?
            .ok_or_else(|| PackageError::InvalidNuspec("<dependency> sin id".into()))?,
        range: attr(e, "version")?,
    })
}

fn attr(e: &BytesStart<'_>, name: &str) -> Result<Option<String>, PackageError> {
    for a in e.attributes() {
        let a = a.map_err(|err| PackageError::InvalidNuspec(err.to_string()))?;
        if a.key.local_name().as_ref() == name {
            let value = a
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|err| PackageError::InvalidNuspec(err.to_string()))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn local_name(e: &BytesStart<'_>) -> String {
    e.local_name().as_ref().to_owned()
}

fn path_str(path: &[String]) -> String {
    path.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn nupkg(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    const NUSPEC: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
  <metadata>
    <id>Hemia.Logging</id>
    <version>1.2.0-Beta.1+sha.abc</version>
    <dependencies>
      <group targetFramework=".NETStandard2.0">
        <dependency id="Hemia.Core" version="[1.0.0, 2.0.0)" exclude="Build,Analyzers" />
      </group>
      <group targetFramework="net8.0" />
    </dependencies>
  </metadata>
</package>"#;

    #[test]
    fn reads_identity_and_dependency_groups() {
        let m = read_package(&nupkg(&[
            ("lib/x.dll", "bin"),
            ("Hemia.Logging.nuspec", NUSPEC),
        ]))
        .unwrap();
        assert_eq!(m.id.as_str(), "Hemia.Logging");
        assert_eq!(m.version.normalized(), "1.2.0-Beta.1");
        assert_eq!(m.version.full(), "1.2.0-Beta.1+sha.abc");
        assert_eq!(
            m.dependency_groups,
            vec![
                DependencyGroup {
                    target_framework: Some(".NETStandard2.0".into()),
                    dependencies: vec![Dependency {
                        id: "Hemia.Core".into(),
                        range: Some("[1.0.0, 2.0.0)".into()),
                    }],
                },
                DependencyGroup {
                    target_framework: Some("net8.0".into()),
                    dependencies: vec![]
                },
            ]
        );
        assert_eq!(m.nuspec, NUSPEC.as_bytes());
    }

    #[test]
    fn legacy_dependencies_without_group() {
        let nuspec = r#"<package><metadata><id>A</id><version>1.0</version>
            <dependencies><dependency id="B" version="1.0" /><dependency id="C" /></dependencies>
            </metadata></package>"#;
        let m = read_package(&nupkg(&[("A.nuspec", nuspec)])).unwrap();
        assert_eq!(m.dependency_groups.len(), 1);
        assert_eq!(m.dependency_groups[0].target_framework, None);
        assert_eq!(m.dependency_groups[0].dependencies.len(), 2);
    }

    #[test]
    fn rejects_malformed_packages() {
        assert!(matches!(
            read_package(b"not a zip"),
            Err(PackageError::NotAZip(_))
        ));
        assert!(matches!(
            read_package(&nupkg(&[("lib/a.dll", "")])),
            Err(PackageError::NuspecMissing)
        ));
        assert!(matches!(
            read_package(&nupkg(&[("sub/A.nuspec", NUSPEC)])),
            Err(PackageError::NuspecMissing)
        ));
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", NUSPEC), ("B.nuspec", NUSPEC)])),
            Err(PackageError::MultipleNuspecs)
        ));
        let bad_version = NUSPEC.replace("1.2.0-Beta.1+sha.abc", "1.0.0-01");
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", &bad_version)])),
            Err(PackageError::InvalidNuspec(_))
        ));
    }

    #[test]
    fn rejects_dtd() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol">]>
            <package><metadata><id>&lol;</id><version>1.0</version></metadata></package>"#;
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", bomb)])),
            Err(PackageError::InvalidNuspec(msg)) if msg.contains("DTD")
        ));
    }

    #[test]
    fn rejects_oversized_nuspec() {
        let huge = format!(
            "<package>{}</package>",
            " ".repeat(InspectionLimits::default().max_nuspec_bytes as usize)
        );
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", &huge)])),
            Err(PackageError::NuspecTooLarge { .. })
        ));
    }

    #[test]
    fn reads_descriptive_metadata() {
        let nuspec = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
  <metadata minClientVersion="2.12">
    <id>Hemia.Rich</id>
    <version>2.0.0-beta.1+build.5</version>
    <title>Hemia Rich</title>
    <authors>Hemia, Cristian</authors>
    <description>Logs &amp; traces &lt;fast&gt; &#233;xito
  second line</description>
    <tags>logging tracing  hemia</tags>
    <license type="expression">MIT</license>
    <licenseUrl>https://aka.ms/deprecateLicenseUrl</licenseUrl>
    <requireLicenseAcceptance>true</requireLicenseAcceptance>
    <icon>icon.png</icon>
    <readme>README.md</readme>
    <projectUrl>https://example.test/rich</projectUrl>
    <repository type="git" url="https://example.test/rich.git" commit="abc" />
    <packageTypes><packageType name="DotnetTool" version="1.0" /></packageTypes>
    <releaseNotes><![CDATA[Fixed <bugs>]]></releaseNotes>
  </metadata>
</package>"#;
        let m = read_package(&nupkg(&[("Hemia.Rich.nuspec", nuspec)])).unwrap();
        let md = &m.metadata;
        assert_eq!(md.title.as_deref(), Some("Hemia Rich"));
        assert_eq!(
            md.description.as_deref(),
            Some("Logs & traces <fast> éxito\n  second line")
        );
        assert_eq!(md.author_list(), ["Hemia", "Cristian"]);
        assert_eq!(md.tag_list(), ["logging", "tracing", "hemia"]);
        assert_eq!(
            md.license,
            Some(License {
                kind: "expression".into(),
                value: "MIT".into()
            })
        );
        assert!(md.require_license_acceptance);
        assert_eq!(md.icon.as_deref(), Some("icon.png"));
        assert_eq!(md.readme.as_deref(), Some("README.md"));
        assert_eq!(
            md.repository_url.as_deref(),
            Some("https://example.test/rich.git")
        );
        assert_eq!(md.min_client_version.as_deref(), Some("2.12"));
        assert_eq!(md.package_type_names(), ["DotnetTool"]);
        assert_eq!(md.release_notes.as_deref(), Some("Fixed <bugs>"));
    }

    #[test]
    fn missing_package_types_default_to_dependency() {
        let m = read_package(&nupkg(&[("A.nuspec", NUSPEC)])).unwrap();
        assert_eq!(m.metadata.package_type_names(), ["Dependency"]);
        assert_eq!(m.metadata.title, None);
    }

    #[test]
    fn unknown_entities_are_rejected() {
        let nuspec = "<package><metadata><id>A</id><version>1.0</version><title>&custom;</title></metadata></package>";
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", nuspec)])),
            Err(PackageError::InvalidNuspec(_))
        ));
    }

    #[test]
    fn rejects_dangerous_paths() {
        for name in [
            "../evil.txt",
            "lib/../../evil.txt",
            "lib\\..\\evil.txt",
            "/etc/passwd",
            "\\server\\share.txt",
            "C:/Windows/evil.dll",
            "lib/file.txt:stream",
            "lib/CON",
            "lib/nul.txt",
            "Com1.dll",
            "lpt9",
            "lib/trailing.",
            "lib/trailing ",
            "lib/%2E%2E/%2E%2E/evil.txt",
            "lib/%2Fabs",
            "lib/bad%zz",
            "lib/a\u{0}b",
            "lib/./a.dll",
            "",
        ] {
            let result = read_package(&nupkg(&[(name, "x"), ("A.nuspec", NUSPEC)]));
            assert!(
                matches!(result, Err(PackageError::UnsafePath(_))),
                "{name:?}: {result:?}"
            );
        }
    }

    #[test]
    fn accepts_ordinary_paths() {
        for name in [
            "lib/net8.0/My%20Lib.dll",
            "content/console.log",
            "[Content_Types].xml",
            "_rels/.rels",
            "package/services/metadata/core-properties/abc.psmdcp",
            "lib/net8.0/",
            "tools/com10.txt",
            "lib/.hidden",
        ] {
            let result = read_package(&nupkg(&[(name, "x"), ("A.nuspec", NUSPEC)]));
            assert!(result.is_ok(), "{name:?}: {result:?}");
        }
    }

    #[test]
    fn rejects_duplicate_entries_ignoring_case_and_escapes() {
        for (a, b) in [("lib/a.dll", "LIB/A.DLL"), ("lib/a b.dll", "lib/a%20b.dll")] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for name in [a, b, "A.nuspec"] {
                zip.start_file(name, SimpleFileOptions::default()).unwrap();
                zip.write_all(NUSPEC.as_bytes()).unwrap();
            }
            let bytes = zip.finish().unwrap().into_inner();
            assert!(matches!(
                read_package(&bytes),
                Err(PackageError::DuplicateEntry(_))
            ));
        }
    }

    fn limits() -> InspectionLimits {
        InspectionLimits {
            max_entries: 3,
            max_entry_bytes: 1000,
            max_total_bytes: 1500,
            ..InspectionLimits::default()
        }
    }

    fn inspect(
        entries: &[(&str, &str)],
        limits: &InspectionLimits,
    ) -> Result<PackageManifest, PackageError> {
        read_package_from(Cursor::new(nupkg(entries)), limits)
    }

    #[test]
    fn enforces_entry_limits() {
        let big = "x".repeat(1001);
        let half = "x".repeat(800);
        assert!(inspect(&[("a", "1"), ("b", "2"), ("A.nuspec", NUSPEC)], &limits()).is_ok());
        assert!(matches!(
            inspect(
                &[("a", "1"), ("b", "2"), ("c", "3"), ("A.nuspec", NUSPEC)],
                &limits()
            ),
            Err(PackageError::TooManyEntries { limit: 3 })
        ));
        assert!(matches!(
            inspect(&[("a", &big), ("A.nuspec", NUSPEC)], &limits()),
            Err(PackageError::EntryTooLarge { .. })
        ));
        assert!(matches!(
            inspect(
                &[("a", &half), ("b", &half), ("A.nuspec", NUSPEC)],
                &limits()
            ),
            Err(PackageError::TooLargeUncompressed { limit: 1500 })
        ));
    }

    #[test]
    fn declared_entry_count_is_read_before_parsing() {
        let bytes = nupkg(&[("a", "1"), ("b", "2"), ("A.nuspec", NUSPEC)]);
        assert_eq!(declared_entries(&mut Cursor::new(&bytes)).unwrap(), Some(3));
        assert_eq!(declared_entries(&mut Cursor::new(b"no zip")).unwrap(), None);
    }

    #[test]
    fn rejects_deep_xml() {
        let depth = InspectionLimits::default().max_xml_depth + 1;
        let nuspec = format!(
            "<package><metadata><id>A</id><version>1.0</version>{}{}</metadata></package>",
            "<x>".repeat(depth),
            "</x>".repeat(depth)
        );
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", &nuspec)])),
            Err(PackageError::XmlTooDeep { .. })
        ));
    }

    #[test]
    fn inspection_has_a_deadline() {
        let limits = InspectionLimits {
            timeout: Duration::ZERO,
            ..InspectionLimits::default()
        };
        assert!(matches!(
            inspect(&[("A.nuspec", NUSPEC)], &limits),
            Err(PackageError::Timeout)
        ));
    }

    #[test]
    fn errors_carry_stable_codes() {
        assert_eq!(PackageError::NuspecMissing.code(), "PACKAGE_INVALID");
        assert_eq!(PackageError::Timeout.code(), "PACKAGE_LIMIT_EXCEEDED");
        assert_eq!(
            PackageError::UnsafePath("..".into()).code(),
            "PACKAGE_UNSAFE_PATH"
        );
        assert!(
            PackageError::Timeout
                .to_string()
                .starts_with("PACKAGE_LIMIT_EXCEEDED: ")
        );
    }
}
