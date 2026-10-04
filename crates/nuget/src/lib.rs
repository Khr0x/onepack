//! Adaptador NuGet: lectura de `.nupkg`, normalización de ids y versiones, recursos V3.

pub mod id;
pub mod package;
pub mod v3;
pub mod version;

pub use id::{InvalidPackageId, PackageId};
pub use package::{
    Dependency, DependencyGroup, License, Metadata, PackageError, PackageManifest, PackageType,
    read_nuspec_from, read_package, read_package_from,
};
pub use version::{InvalidVersion, NuGetVersion};
