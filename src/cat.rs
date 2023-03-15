use std::{path::Path, fs::File};

use crate::error::UtilError;
use rust_car::reader as car_reader;

pub(crate) fn cat_content(path: impl AsRef<Path>, cid: &str) -> Result<(), UtilError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(UtilError::new(format!(
            "car file [{}] is not exist.",
            path.to_str().unwrap()
        )));
    }
    let file = File::open(path)?;
    let mut reader = car_reader::new_v1(file)?;
    rust_car::utils::cat_ipld_str(&mut reader, cid)?;
    Ok(())
}