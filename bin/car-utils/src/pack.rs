use crate::error::UtilError;
use blockless_car::utils::archive_local;
use std::path::Path;

#[derive(Debug, clap::Parser)]
pub struct PackCommand {
    /// The source file or directory to be packed.
    source: String,

    #[clap(
        help = "Wrap the file (applies to files only).",
        default_value = "false",
        long = "no-wrap"
    )]
    no_wrap_file: bool,

    #[clap(short, help = "The car file to output.")]
    output: String,
}

impl PackCommand {
    /// archive the local file system to car file
    /// `target` is the car file
    /// `source` is the directory where the archive is prepared.
    pub(crate) fn execute(&self) -> Result<(), UtilError> {
        let file = std::fs::File::create(self.output.as_ref() as &Path).unwrap(); // todo handle error
        archive_local(self.source.as_ref() as &Path, file, self.no_wrap_file)?;
        Ok(())
    }
}
