use std::path::{Path, PathBuf};

/// Convert Windows verbatim paths like `\\?\C:\repo` back to normal user paths.
///
/// `std::fs::canonicalize` can return verbatim paths on Windows. They are useful
/// for Win32 APIs, but PowerShell shows them as provider paths in prompts.
pub fn user_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        return windows_user_path(path);
    }

    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

#[cfg(any(windows, test))]
fn windows_user_path(path: &Path) -> PathBuf {
    let path_text = path.to_string_lossy();
    if let Some(stripped) = path_text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{stripped}"))
    } else if let Some(stripped) = path_text.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

pub fn user_path_string(path: &Path) -> String {
    user_path(path).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_windows_verbatim_drive_prefix() {
        assert_eq!(
            windows_user_path(Path::new(r"\\?\C:\Users\zedra\zedra")),
            PathBuf::from(r"C:\Users\zedra\zedra")
        );
    }

    #[test]
    fn strips_windows_verbatim_unc_prefix() {
        assert_eq!(
            windows_user_path(Path::new(r"\\?\UNC\server\share\repo")),
            PathBuf::from(r"\\server\share\repo")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn user_path_is_noop_off_windows() {
        let path = Path::new("/tmp/repo");
        assert_eq!(user_path(path), PathBuf::from("/tmp/repo"));
    }
}
