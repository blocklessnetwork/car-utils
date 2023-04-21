use std::{fs::File, path::Path};

use crate::error::UtilError;
use blockless_car::reader::{self as car_reader, CarReader};
use blockless_car::utils::extract_ipld;

/// extract car file to local file system.
/// `car` the car file for extract.
/// `target` target directory to extract.
pub(crate) fn extract_car(car: impl AsRef<Path>, target: Option<&String>) -> Result<(), UtilError> {
    let path = car.as_ref();
    if !path.exists() {
        return Err(UtilError::new(format!(
            "car file [{}] is not exist.",
            path.to_str().unwrap()
        )));
    }
    let file = File::open(path)?;
    let mut reader = car_reader::new_v1(file)?;
    let roots = reader.header().roots();
    for cid in roots {
        extract_ipld(&mut reader, cid, target)?;
    }
    Ok(())
}
