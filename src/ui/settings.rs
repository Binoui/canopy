use std::{fs, io, path::Path};

pub(super) fn write(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let result = (|| {
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
