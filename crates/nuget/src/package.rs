//! Lectura de `.nupkg`: localiza el `.nuspec` en la raíz del ZIP y extrae la identidad y las
//! dependencias. No extrae nada a disco ni ejecuta contenido del paquete (ADR-014).

use std::io::{Cursor, Read, Seek};

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use crate::{NuGetVersion, PackageId};

/// Tamaño máximo del `.nuspec` descomprimido. Los límites completos llegan en la Fase 5.
pub const MAX_NUSPEC_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct PackageManifest {
    pub id: PackageId,
    pub version: NuGetVersion,
    pub dependency_groups: Vec<DependencyGroup>,
    /// Bytes originales del `.nuspec`, para servirlos sin regenerarlos.
    pub nuspec: Vec<u8>,
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

#[derive(Debug)]
pub enum PackageError {
    NotAZip(zip::result::ZipError),
    NuspecMissing,
    MultipleNuspecs,
    NuspecTooLarge,
    InvalidNuspec(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAZip(e) => write!(f, "el paquete no es un ZIP válido: {e}"),
            Self::NuspecMissing => f.write_str("el paquete no contiene un .nuspec en la raíz"),
            Self::MultipleNuspecs => {
                f.write_str("el paquete contiene más de un .nuspec en la raíz")
            }
            Self::NuspecTooLarge => write!(f, "el .nuspec supera {MAX_NUSPEC_BYTES} bytes"),
            Self::InvalidNuspec(msg) => write!(f, ".nuspec inválido: {msg}"),
        }
    }
}

impl std::error::Error for PackageError {}

pub fn read_package(nupkg: &[u8]) -> Result<PackageManifest, PackageError> {
    read_package_from(Cursor::new(nupkg))
}

/// Lee el manifiesto sin cargar el paquete completo en memoria (p. ej. desde un archivo).
pub fn read_package_from<R: Read + Seek>(nupkg: R) -> Result<PackageManifest, PackageError> {
    parse_nuspec(read_nuspec_from(nupkg)?)
}

/// Devuelve solo los bytes del `.nuspec` (para servir `/{id}/{version}/{id}.nuspec`).
pub fn read_nuspec_bytes(nupkg: &[u8]) -> Result<Vec<u8>, PackageError> {
    read_nuspec_from(Cursor::new(nupkg))
}

pub fn read_nuspec_from<R: Read + Seek>(nupkg: R) -> Result<Vec<u8>, PackageError> {
    let mut archive = zip::ZipArchive::new(nupkg).map_err(PackageError::NotAZip)?;

    let mut found = None;
    for i in 0..archive.len() {
        let name = archive.name_for_index(i).unwrap_or_default();
        if !name.contains('/') && name.to_ascii_lowercase().ends_with(".nuspec") {
            if found.is_some() {
                return Err(PackageError::MultipleNuspecs);
            }
            found = Some(i);
        }
    }
    let index = found.ok_or(PackageError::NuspecMissing)?;

    let entry = archive.by_index(index).map_err(PackageError::NotAZip)?;
    let mut bytes = Vec::new();
    entry
        .take(MAX_NUSPEC_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PackageError::InvalidNuspec(e.to_string()))?;
    if bytes.len() as u64 > MAX_NUSPEC_BYTES {
        return Err(PackageError::NuspecTooLarge);
    }
    Ok(bytes)
}

pub fn parse_nuspec(nuspec: Vec<u8>) -> Result<PackageManifest, PackageError> {
    let invalid = |msg: String| PackageError::InvalidNuspec(msg);
    let mut reader = Reader::from_reader(nuspec.as_slice());
    reader.config_mut().trim_text(true);

    let mut path: Vec<String> = Vec::new();
    let mut id = None;
    let mut version = None;
    let mut groups: Vec<DependencyGroup> = Vec::new();
    let mut buf = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buf)
            .map_err(|e| invalid(e.to_string()))?;
        match event {
            Event::DocType(_) => return Err(invalid("DTD no permitido".into())),
            Event::Start(e) => {
                on_element(&e, &path, &mut groups)?;
                path.push(local_name(&e));
            }
            Event::Empty(e) => on_element(&e, &path, &mut groups)?,
            Event::End(_) => {
                path.pop();
            }
            Event::Text(t) => {
                let text = t.xml10_content();
                match path_str(&path).as_str() {
                    "package/metadata/id" => id = Some(text.trim().to_owned()),
                    "package/metadata/version" => version = Some(text.trim().to_owned()),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let id = id.ok_or_else(|| invalid("falta <id>".into()))?;
    let version = version.ok_or_else(|| invalid("falta <version>".into()))?;
    Ok(PackageManifest {
        id: PackageId::parse(&id).map_err(|e| invalid(e.to_string()))?,
        version: NuGetVersion::parse(&version).map_err(|e| invalid(e.to_string()))?,
        dependency_groups: groups,
        nuspec,
    })
}

fn on_element(
    e: &BytesStart<'_>,
    parent: &[String],
    groups: &mut Vec<DependencyGroup>,
) -> Result<(), PackageError> {
    let name = local_name(e);
    match (path_str(parent).as_str(), name.as_str()) {
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
            " ".repeat(MAX_NUSPEC_BYTES as usize)
        );
        assert!(matches!(
            read_package(&nupkg(&[("A.nuspec", &huge)])),
            Err(PackageError::NuspecTooLarge)
        ));
    }
}
