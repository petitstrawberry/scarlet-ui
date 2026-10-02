//! Capabilities of Scarlet Files, separate from its sbus transport.
use scarlet_ui_core::file_dialog::{
    FileDialog, FileDialogError, FileDialogFilterPolicy, FileDialogMode,
};

pub(crate) fn native_filter(options: &FileDialog) -> Result<&'static str, FileDialogError> {
    options.validate()?;
    if options.mode == FileDialogMode::OpenMultiple {
        return Err(FileDialogError::Unsupported(
            "Files returns only one path despite allow_multiple".into(),
        ));
    }
    if !options.filters.is_empty() && options.filter_policy == FileDialogFilterPolicy::Required {
        return Err(FileDialogError::Unsupported(
            "Files cannot enforce arbitrary extension filters; use Optional and validate selections".into(),
        ));
    }
    // Files supports only a small fixed MIME list, including no audio wildcard.
    // Do not disguise arbitrary extension lists as a supported MIME filter.
    Ok("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::file_dialog::FileDialogFilter;

    #[test]
    fn filtered_single_open_and_save_can_opt_into_files() {
        for mode in [FileDialogMode::Open, FileDialogMode::Save] {
            let mut options = FileDialog::new(mode);
            options.filters.push(FileDialogFilter {
                name: "WAV audio".into(),
                extensions: vec!["wav".into()],
            });
            assert!(matches!(
                native_filter(&options),
                Err(FileDialogError::Unsupported(_))
            ));
            options.filter_policy = FileDialogFilterPolicy::Optional;
            assert_eq!(native_filter(&options), Ok(""));
        }
    }
    #[test]
    fn optional_filters_do_not_relax_multiple_selection_or_invalid_options() {
        let mut options = FileDialog::new(FileDialogMode::OpenMultiple);
        options.filter_policy = FileDialogFilterPolicy::Optional;
        assert!(matches!(
            native_filter(&options),
            Err(FileDialogError::Unsupported(_))
        ));
        options.mode = FileDialogMode::Save;
        options.default_name = Some("../escape.wav".into());
        assert!(matches!(
            native_filter(&options),
            Err(FileDialogError::InvalidOptions(_))
        ));
        assert_eq!(
            native_filter(&FileDialog::new(FileDialogMode::Open)),
            Ok("")
        );
    }
}
