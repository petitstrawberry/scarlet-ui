//! Files extension filtering with explicit provider capability negotiation.
use alloc::{string::String, vec::Vec};
use scarlet_ui_core::file_dialog::{
    FileDialog, FileDialogError, FileDialogFilterPolicy, FileDialogMode,
};

// New Files service API, implemented in Scarlet alongside the existing MIME filter.
// Kept here so older pinned runtime bindings remain ABI-compatible.
pub(crate) const CAPABILITIES_METHOD: &str = "GetPickerCapabilities";
pub(crate) const EXTENSION_CAPABILITY: &str = "extension-list-v1";

pub(crate) fn native_filter(
    options: &FileDialog,
    extensions_supported: bool,
) -> Result<String, FileDialogError> {
    options.validate()?;
    if options.mode == FileDialogMode::OpenMultiple {
        return Err(FileDialogError::Unsupported(
            "Files returns only one path despite allow_multiple".into(),
        ));
    }
    if options.filters.is_empty() {
        return Ok(String::new());
    }
    if !extensions_supported {
        return if options.filter_policy == FileDialogFilterPolicy::Optional {
            Ok(String::new())
        } else {
            Err(FileDialogError::Unsupported(
                "Files has no extension-list-v1 capability".into(),
            ))
        };
    }
    let mut extensions: Vec<String> = Vec::new();
    for extension in options.filters.iter().flat_map(|filter| &filter.extensions) {
        let extension = extension.to_ascii_lowercase();
        if !extensions.contains(&extension) {
            extensions.push(extension);
        }
    }
    Ok(alloc::format!("extensions:{}", extensions.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::file_dialog::FileDialogFilter;
    #[test]
    fn filtered_single_open_and_save_use_the_union() {
        for mode in [FileDialogMode::Open, FileDialogMode::Save] {
            let mut options = FileDialog::new(mode);
            options.filters = vec![
                FileDialogFilter {
                    name: "Audio".into(),
                    extensions: vec!["WAV".into(), "flac".into()],
                },
                FileDialogFilter {
                    name: "Project and audio".into(),
                    extensions: vec!["json".into(), "wav".into()],
                },
            ];
            assert_eq!(
                native_filter(&options, true),
                Ok("extensions:wav,flac,json".into())
            );
            assert!(matches!(
                native_filter(&options, false),
                Err(FileDialogError::Unsupported(_))
            ));
            options.filter_policy = FileDialogFilterPolicy::Optional;
            assert_eq!(native_filter(&options, false), Ok(String::new()));
            assert_eq!(
                native_filter(&options, true),
                Ok("extensions:wav,flac,json".into())
            );
        }
    }
    #[test]
    fn filters_do_not_relax_multiple_selection_or_invalid_options() {
        let mut options = FileDialog::new(FileDialogMode::OpenMultiple);
        options.filter_policy = FileDialogFilterPolicy::Optional;
        assert!(matches!(
            native_filter(&options, true),
            Err(FileDialogError::Unsupported(_))
        ));
        options.mode = FileDialogMode::Save;
        options.default_name = Some("../escape.wav".into());
        assert!(matches!(
            native_filter(&options, true),
            Err(FileDialogError::InvalidOptions(_))
        ));
        options.default_name = None;
        options.filters = vec![FileDialogFilter {
            name: "Invalid".into(),
            extensions: vec!["wav,txt".into()],
        }];
        assert!(matches!(
            native_filter(&options, true),
            Err(FileDialogError::InvalidOptions(_))
        ));
        assert_eq!(
            native_filter(&FileDialog::new(FileDialogMode::Open), false),
            Ok(String::new())
        );
    }
}
