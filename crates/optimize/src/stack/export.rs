//! What an export writes, and how.

use serde::{Deserialize, Serialize};

/// How the LOD chain is laid out on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LodPackaging {
    /// One FBX holding `MeshName_LOD0` … `MeshName_LODn` as sibling nodes — the
    /// naming convention most engines auto-detect.
    #[default]
    SingleFileSuffixed,
    /// `Asset_LOD0.fbx`, `Asset_LOD1.fbx`, … one file per level.
    FilePerLod,
}

impl LodPackaging {
    pub const ALL: [LodPackaging; 2] = [LodPackaging::SingleFileSuffixed, LodPackaging::FilePerLod];

    pub fn label(self) -> &'static str {
        match self {
            LodPackaging::SingleFileSuffixed => "Single file, suffixed nodes",
            LodPackaging::FilePerLod => "One file per LOD",
        }
    }
}

/// Whether the export reconstructs the source scene graph or flattens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HierarchyMode {
    /// Re-emit the original node hierarchy, moving geometry back into each
    /// node's local space. The export round-trips like the source asset.
    #[default]
    Rebuild,
    /// Emit one root-level node per mesh with world-space geometry and an
    /// identity transform.
    FlatBaked,
}

impl HierarchyMode {
    pub const ALL: [HierarchyMode; 2] = [HierarchyMode::Rebuild, HierarchyMode::FlatBaked];

    pub fn label(self) -> &'static str {
        match self {
            HierarchyMode::Rebuild => "Rebuild original hierarchy",
            HierarchyMode::FlatBaked => "Flat, world-baked meshes",
        }
    }
}

/// FBX container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FbxFormat {
    #[default]
    Binary,
    Ascii,
}

impl FbxFormat {
    pub const ALL: [FbxFormat; 2] = [FbxFormat::Binary, FbxFormat::Ascii];

    pub fn label(self) -> &'static str {
        match self {
            FbxFormat::Binary => "Binary",
            FbxFormat::Ascii => "ASCII",
        }
    }
}

/// What kind of file an export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportFileType {
    #[default]
    Fbx,
    /// Binary glTF 2.0.
    Glb,
}

impl ExportFileType {
    pub const ALL: [ExportFileType; 2] = [ExportFileType::Fbx, ExportFileType::Glb];

    pub fn label(self) -> &'static str {
        match self {
            ExportFileType::Fbx => "FBX",
            ExportFileType::Glb => "glb (binary glTF)",
        }
    }

    /// The file extension, lower case, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFileType::Fbx => "fbx",
            ExportFileType::Glb => "glb",
        }
    }
}

/// Settings for the export step, edited through the stack panel's
/// "Export settings" row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    pub file_type: ExportFileType,
    pub packaging: LodPackaging,
    pub hierarchy: HierarchyMode,
    /// The FBX encoding; a glb is binary by definition.
    pub format: FbxFormat,
    /// Leave the custom vertex attributes (glTF's `_`-prefixed ones) out of a
    /// glb: what a pipeline needs while it works is not always what the game
    /// should load.
    pub strip_custom_attributes: bool,
}
