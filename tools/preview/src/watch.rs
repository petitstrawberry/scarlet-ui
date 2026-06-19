use std::fs;
use std::path::Path;
use std::time::SystemTime;

pub(crate) fn latest_source_mtime(root: &Path) -> Result<SystemTime, String> {
    let mut latest = SystemTime::UNIX_EPOCH;
    visit_sources(root, &mut |path| {
        if let Ok(metadata) = fs::metadata(path)
            && let Ok(modified) = metadata.modified()
            && modified > latest
        {
            latest = modified;
        }
    })?;
    Ok(latest)
}

pub(crate) fn visit_sources(root: &Path, visitor: &mut impl FnMut(&Path)) -> Result<(), String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("failed to read directory {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name == "target" || name == ".git" || name == ".scarlet-ui-preview" {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to stat {}: {error}", path.display()))?;
        if metadata.is_dir() {
            visit_sources(&path, visitor)?;
        } else if is_source_file(&path) {
            visitor(&path);
        }
    }
    Ok(())
}

fn is_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("rs" | "toml" | "lock")
    )
}
