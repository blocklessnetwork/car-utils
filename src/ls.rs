use rust_car::reader as car_reader;
use rust_car::utils;
use std::fs::File;
use std::path::Path;

use crate::error::UtilError;

/// list files from car file.
/// `path` is the car file path.
pub(crate) fn list_car_file(path: impl AsRef<Path>) -> Result<(), UtilError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(UtilError::new(format!("car file [{}] is not exist.", path.to_str().unwrap())));
    }
    let file = File::open(path)?;
    let mut reader = car_reader::new_v1(file)?;
    utils::list(&mut reader);
    Ok(())
}
