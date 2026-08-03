//! Embedded file-type artwork from file-icon-vectors.
//!
//! The upstream SVG files are converted at build time into colored triangle
//! meshes. Scarlet UI can therefore draw file icons without a runtime SVG
//! parser or a filesystem lookup.

#![no_std]

/// One filled triangle in an embedded file icon mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileIconTriangle {
    /// First vertex in the source view box.
    pub a: [f32; 2],
    /// Second vertex in the source view box.
    pub b: [f32; 2],
    /// Third vertex in the source view box.
    pub c: [f32; 2],
    /// Opaque or translucent RGBA color packed as `0xRRGGBBAA`.
    pub color: u32,
}

/// Vector dimensions and tessellated artwork for one file icon.
#[derive(Clone, Copy, Debug)]
pub struct FileIconData {
    /// Source view-box width.
    pub width: f32,
    /// Source view-box height.
    pub height: f32,
    /// Triangles in source coordinate space and paint order.
    pub triangles: &'static [FileIconTriangle],
}

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

#[cfg(test)]
mod tests {
    use super::{FileIcon, extra_folder_icon, file_icon_data, vivid_icon_for_extension};

    #[test]
    fn standard_assets_have_meshes() {
        assert!(!file_icon_data(extra_folder_icon()).triangles.is_empty());
        assert!(!file_icon_data(FileIcon::VividImage).triangles.is_empty());
    }

    #[test]
    fn extension_lookup_uses_vivid_assets() {
        assert_eq!(vivid_icon_for_extension("png"), Some(FileIcon::VividPng));
        assert_eq!(vivid_icon_for_extension("unknown-extension"), None);
    }
}
