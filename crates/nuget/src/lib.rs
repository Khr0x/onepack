//! Adaptador NuGet: lectura de `.nupkg`, normalización de ids y versiones, recursos V3.

pub mod id;
pub mod package;
pub mod version;

pub use id::{InvalidPackageId, PackageId};
pub use package::{Dependency, DependencyGroup, PackageError, PackageManifest, read_package};
pub use version::{InvalidVersion, NuGetVersion};
