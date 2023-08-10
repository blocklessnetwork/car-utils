use std::{fs::File, path::Path};

use crate::error::UtilError;
use blockless_car::reader::{self as car_reader, CarReader};
use blockless_car::utils::extract_ipld;

#[derive(Debug, clap::Parser)]
pub struct UnpackCommand {
    #[clap(short, help = "The car file to extract")]
    car: String,

    #[clap(short, help = "Target directory to extract to")]
    target: Option<String>,
}

impl UnpackCommand {
    /// extract car file to local file system.
    /// `car` the car file to extract.
    /// `target` target directory to extract.
    pub(crate) fn execute(&self) -> Result<(), UtilError> {
        let path: &Path = self.car.as_ref();
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
            let target: Option<&Path> = self.target.as_ref().map(|s| s.as_ref());
            extract_ipld(&mut reader, cid, target)?;
        }
        Ok(())
    }
}
