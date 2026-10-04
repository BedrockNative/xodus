use std::path::{Path, PathBuf};

const APP_ID: &str = "io.github.BedrockNative.Xodus";

pub fn prepare() {
    // GTK3 uses the program name as the Wayland app_id when no GtkApplication
    // ID is set. Keep separate login processes independent (no unique app bus).
    glib::set_prgname(Some(APP_ID));
    glib::set_application_name("Xodus");
    if let Err(error) = install_metadata() {
        // An unwritable data directory must not prevent signing in.
        tracing::warn!(%error, "Could not install the login window's desktop icon");
    }
}

fn install_metadata() -> std::io::Result<()> {
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".local/share"))
        })
        .ok_or_else(|| std::io::Error::other("No absolute XDG_DATA_HOME or HOME"))?;

    write_if_changed(
        &data_dir.join(format!("icons/hicolor/48x48/apps/{APP_ID}.png")),
        include_bytes!("../../../assets/Xbox/appicon.png"),
    )?;
    write_if_changed(
        &data_dir.join(format!("applications/{APP_ID}.desktop")),
        include_bytes!("../../../assets/io.github.BedrockNative.Xodus.desktop"),
    )
}

fn write_if_changed(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|existing| existing == contents) {
        return Ok(());
    }
    let directory = path.parent().expect("desktop asset has a parent");
    std::fs::create_dir_all(directory)?;
    // Atomic replacement also allows two independent login windows to start.
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    std::io::Write::write_all(&mut file, contents)?;
    file.persist(path)?;
    Ok(())
}
