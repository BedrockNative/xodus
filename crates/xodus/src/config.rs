use std::path::PathBuf;

/// Resolve an optional profile directory shared by the CLI and service.
/// Canonical paths give equivalent paths (including symlinks) the same scope.
pub fn directory() -> std::io::Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os("XODUS_CONFIG_DIR").filter(|path| !path.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&path)?;
    path.canonicalize().map(Some)
}
