use crate::error::UtilError;
use blockless_car::reader as car_reader;
use std::{fs::File, path::Path};

#[derive(Debug, clap::Parser)]
pub struct CatCommand {
    #[clap(help = "the car file to cat.")]
    car: String,

    #[clap(short, help = "the cid of content to cat.")]
    cid: String,
}

impl CatCommand {
    pub(crate) fn execute(&self) -> Result<(), UtilError> {
        let path: &Path = self.car.as_ref();
        if !path.exists() {
            return Err(UtilError::new(format!(
                "the car file [{}] does not exist.",
                self.car
            )));
        }
        let file = File::open(path)?;
        let mut reader = car_reader::new_v1(file)?;
        blockless_car::utils::cat_ipld_str(&mut reader, &self.cid)?;
        Ok(())
    }
}
