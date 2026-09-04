//! Platform-aware discovery of fonts used by the default UI font stack.

#[cfg(target_os = "scarlet")]
use alloc::sync::Arc;
use alloc::vec::Vec;
#[cfg(target_os = "scarlet")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(target_os = "scarlet")]
use fontique::Blob;
use fontique::{
    Collection, CollectionOptions, FallbackKey, FamilyId, GenericFamily, QueryStatus, Script,
    SourceCache,
};
use unicode_script::UnicodeScript;

/// Owned font data selected for a character.
pub(super) struct DiscoveredFont {
    pub(super) bytes: Vec<u8>,
    pub(super) index: u32,
}

/// Font database used to add default-stack fallbacks only when they are needed.
#[derive(Clone)]
pub(super) struct FontDiscovery {
    collection: Collection,
    source_cache: SourceCache,
    loaded_faces: Vec<(FamilyId, usize)>,
    unresolved_chars: Vec<char>,
}

impl FontDiscovery {
    /// Discover the fonts exposed by the current platform.
    pub(super) fn new() -> Self {
        let mut collection = platform_collection();
        let configured_paths = configured_font_paths();

        #[cfg(not(target_os = "scarlet"))]
        collection.load_fonts_from_paths(&configured_paths);

        #[cfg(target_os = "scarlet")]
        {
            for path in configured_paths {
                register_font_path(&mut collection, &path, 0);
            }
            register_scarlet_generic_families(&mut collection);
        }

        Self {
            collection,
            source_cache: SourceCache::default(),
            loaded_faces: Vec::new(),
            unresolved_chars: Vec::new(),
        }
    }

    /// Select and own a font face that contains `ch`.
    pub(super) fn font_for_char(&mut self, ch: char) -> Option<DiscoveredFont> {
        if self.unresolved_chars.contains(&ch) {
            return None;
        }

        let script_name = ch.script().short_name().as_bytes();
        let script = Script::from_bytes(script_name.try_into().ok()?);
        let loaded_faces = &self.loaded_faces;
        let mut selected = None;
        {
            let mut query = self.collection.query(&mut self.source_cache);
            query.set_families([
                GenericFamily::SystemUi,
                GenericFamily::SansSerif,
                GenericFamily::Serif,
                GenericFamily::Monospace,
                GenericFamily::Emoji,
                GenericFamily::Math,
            ]);
            query.set_fallbacks(FallbackKey::new(script, None));
            query.matches_with(|font| {
                if loaded_faces.contains(&font.family)
                    || !font
                        .charmap()
                        .is_some_and(|charmap| charmap.map(ch).is_some())
                {
                    return QueryStatus::Continue;
                }
                selected = Some((font.family, font.blob.as_ref().to_vec(), font.index));
                QueryStatus::Stop
            });
        }

        let Some((face, bytes, index)) = selected else {
            self.unresolved_chars.push(ch);
            return None;
        };
        self.loaded_faces.push(face);
        Some(DiscoveredFont { bytes, index })
    }
}

#[cfg(not(target_os = "scarlet"))]
fn platform_collection() -> Collection {
    Collection::new(CollectionOptions::default())
}

#[cfg(target_os = "scarlet")]
fn platform_collection() -> Collection {
    Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    })
}

fn configured_font_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::env::var_os("SCARLET_UI_FONT_PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();

    #[cfg(target_os = "scarlet")]
    paths.extend([
        PathBuf::from("/fonts"),
        PathBuf::from("/system/share/fonts"),
    ]);

    paths.sort_unstable();
    paths.dedup();
    paths
}

#[cfg(target_os = "scarlet")]
fn register_font_path(collection: &mut Collection, path: &Path, depth: u8) {
    const MAX_DEPTH: u8 = 16;

    if depth > MAX_DEPTH {
        return;
    }
    let Ok(metadata) = path.metadata() else {
        return;
    };
    if metadata.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        children.sort_unstable();
        for child in children {
            register_font_path(collection, &child, depth.saturating_add(1));
        }
        return;
    }
    if !is_font_path(path) {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    collection.register_fonts(Blob::new(Arc::new(bytes) as _), None);
}

#[cfg(target_os = "scarlet")]
fn is_font_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "ttc" | "otf" | "otc"
            )
        })
}

#[cfg(target_os = "scarlet")]
fn register_scarlet_generic_families(collection: &mut Collection) {
    let family_names: Vec<_> = collection.family_names().map(str::to_owned).collect();
    let family_ids: Vec<_> = family_names
        .iter()
        .filter_map(|name| collection.family_id(name))
        .collect();
    for generic in [
        GenericFamily::SystemUi,
        GenericFamily::SansSerif,
        GenericFamily::Serif,
        GenericFamily::Monospace,
        GenericFamily::Emoji,
        GenericFamily::Math,
    ] {
        collection.append_generic_families(generic, family_ids.iter().copied());
    }
}

#[cfg(test)]
mod tests {
    use ab_glyph::{Font as _, FontRef};

    use super::FontDiscovery;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_fallback_contains_japanese_glyphs() {
        let mut discovery = FontDiscovery::new();
        let font = discovery
            .font_for_char('日')
            .expect("macOS should expose a Japanese system fallback font");
        let font = FontRef::try_from_slice_and_index(&font.bytes, font.index)
            .expect("the discovered system font should be valid");
        assert_ne!(font.glyph_id('日').0, 0);
    }
}
