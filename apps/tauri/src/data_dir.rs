use std::{ffi::OsString, path::PathBuf};

/// A packaged or mobile app must never follow a development data-directory override.
pub fn development_override(value: Option<OsString>) -> Result<Option<PathBuf>, String> {
    #[cfg(all(debug_assertions, desktop, not(feature = "custom-protocol")))]
    {
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err("WF_DATA_DIR must be an absolute path (do not use ~).".into());
        }
        Ok(Some(path))
    }
    #[cfg(not(all(debug_assertions, desktop, not(feature = "custom-protocol"))))]
    {
        let _ = value;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_uses_app_data() {
        assert_eq!(development_override(None), Ok(None));
        assert_eq!(development_override(Some(OsString::new())), Ok(None));
    }

    #[test]
    fn override_is_only_enabled_for_desktop_development() {
        let path = std::env::temp_dir().join("wealthfolio-dev");
        let result = development_override(Some(path.clone().into_os_string()));
        if cfg!(all(
            debug_assertions,
            desktop,
            not(feature = "custom-protocol")
        )) {
            assert_eq!(result, Ok(Some(path)));
            assert!(development_override(Some("./dev-data".into())).is_err());
            assert!(development_override(Some("~/dev-data".into())).is_err());
        } else {
            assert_eq!(result, Ok(None));
            assert_eq!(development_override(Some("./dev-data".into())), Ok(None));
        }
    }
}
