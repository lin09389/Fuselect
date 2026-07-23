use std::env;
use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use crate::APPLICATION_NAME;

/// Resolve the Fuselect data directory.
///
/// Prefer `FUSELECT_HOME` so tests and operators never touch the real user profile
/// unless they explicitly choose to.
pub fn resolve_data_dir() -> PathBuf {
    resolve_data_dir_from(
        env::var_os("FUSELECT_HOME").as_deref(),
        dirs::data_dir().as_deref(),
    )
}

fn resolve_data_dir_from(home: Option<&OsStr>, platform_data_dir: Option<&Path>) -> PathBuf {
    if let Some(home) = home.filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }

    platform_data_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APPLICATION_NAME)
}

pub fn database_path() -> PathBuf {
    resolve_data_dir().join("metadata.sqlite")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuselect_home_overrides_platform_dir() {
        let resolved = resolve_data_dir_from(
            Some(OsStr::new(r"C:\tmp\fuselect-test-home")),
            Some(Path::new(r"C:\Users\example\AppData\Local")),
        );
        assert_eq!(resolved, PathBuf::from(r"C:\tmp\fuselect-test-home"));
    }

    #[test]
    fn platform_data_dir_is_used_when_override_is_absent() {
        let resolved =
            resolve_data_dir_from(None, Some(Path::new(r"C:\Users\example\AppData\Local")));
        assert_eq!(
            resolved,
            PathBuf::from(r"C:\Users\example\AppData\Local\fuselect")
        );
    }
}
