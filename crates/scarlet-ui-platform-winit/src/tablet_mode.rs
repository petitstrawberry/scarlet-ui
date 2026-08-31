//! Parsing for the platform-independent tablet-mode startup override.

pub(crate) fn parse_tablet_mode_override(value: Option<&str>) -> Option<bool> {
    let value = value?.trim();
    if ["1", "true", "yes", "on", "tablet"]
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
    {
        Some(true)
    } else if ["0", "false", "no", "off", "laptop"]
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
    {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_is_case_insensitive_and_rejects_invalid_values() {
        for value in ["1", "TRUE", "Yes", "on", "TaBlEt"] {
            assert_eq!(parse_tablet_mode_override(Some(value)), Some(true));
        }
        for value in ["0", "FALSE", "No", "off", "LaPtOp"] {
            assert_eq!(parse_tablet_mode_override(Some(value)), Some(false));
        }
        assert_eq!(parse_tablet_mode_override(Some("maybe")), None);
        assert_eq!(parse_tablet_mode_override(None), None);
    }
}
