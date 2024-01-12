use crate::error::UtilError;
use blockless_car::utils::pack_files;
use std::path::Path;

#[allow(non_camel_case_types)]
#[derive(clap::ValueEnum, Clone, Debug)]
enum HasherCodec {
    Sha2_256,
    Blake2b_256,
}

#[derive(Debug, clap::Parser)]
pub struct PackCommand {
    /// The source file or directory to be packed.
    source: String,

    #[clap(
        value_enum,
        help = "The hashing algorithm to use",
        default_value = "sha2-256"
    )]
    hasher_codec: HasherCodec,

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
        let hasher_codec = match self.hasher_codec {
            HasherCodec::Sha2_256 => multicodec::Codec::Sha2_256,
            HasherCodec::Blake2b_256 => multicodec::Codec::Blake2b_256,
        };
        pack_files(
            self.source.as_ref() as &Path,
            file,
            hasher_codec,
            self.no_wrap_file,
        )?;
        Ok(())
    }
}
