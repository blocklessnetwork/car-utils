use blockless_car::utils::archive_local;
use std::path::Path;

use crate::error::UtilError;

/// archive the local file system to car file
/// `target` is the car file
/// `source` is the directory where the archive is prepared.
pub(crate) fn archive_local_fs(
    target: impl AsRef<Path>,
    source: impl AsRef<Path>,
) -> Result<(), UtilError> {
    let target = target.as_ref();
    let file = std::fs::File::create(target).unwrap();
    archive_local(source, file)?;
    Ok(())
}
