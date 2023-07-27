use blockless_car::reader as car_reader;
use blockless_car::utils;
use std::fs::File;
use std::path::Path;

use crate::error::UtilError;

#[derive(Debug, clap::Parser)]
pub struct LsCommand {
    #[clap(help = "the car file for list.")]
    car: String,
}

impl LsCommand {
    /// list files from car file.
    /// `path` is the car file path.
    pub(crate) fn execute(&self, is_cid: bool) -> Result<(), UtilError> {
        // Ok(list_car_file(&self.car, is_cid)?)
        let path: &Path = self.car.as_ref();
        if !path.exists() {
            return Err(UtilError::new(format!(
                "car file [{}] is not exist.",
                path.to_str().unwrap()
            )));
        }
        let file = File::open(path)?;
        let mut reader = car_reader::new_v1(file)?;
        if is_cid {
            utils::list_cid(&mut reader)?;
        } else {
            utils::list(&mut reader)?;
        }
        Ok(())
    }
}
