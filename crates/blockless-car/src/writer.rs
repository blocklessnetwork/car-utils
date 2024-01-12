use cid::Cid;
use ipld::{pb::DagPbCodec, prelude::Codec};

use crate::{
    error::CarError,
    utils::{empty_pb_cid, pb_cid},
    CarHeader, Ipld,
};

mod writer_v1;
pub(crate) use writer_v1::CarWriterV1;

pub enum WriteStream<'bs> {
    Bytes(&'bs [u8]),
    End,
}

pub trait CarWriter {
    fn write_block<T>(&mut self, cid: Cid, data: T) -> Result<(), CarError>
    where
        T: AsRef<[u8]>;

    fn stream_block<F, R>(
        &mut self,
        cid_f: F,
        stream_len: usize,
        r: &mut R,
    ) -> Result<Cid, CarError>
    where
        R: std::io::Read + std::io::Seek,
        F: FnMut(WriteStream) -> Option<Result<Cid, CarError>>;

    fn write_ipld(&mut self, ipld: Ipld, hasher_codec: multicodec::Codec) -> Result<Cid, CarError> {
        match ipld {
            Ipld::Bytes(buf) => {
                let file_cid = crate::utils::raw_cid(&buf, hasher_codec);
                self.write_block(file_cid, &buf)?;
                Ok(file_cid)
            }
            fs_ipld @ ipld::Ipld::Map(_) => {
                let bs: Vec<u8> = DagPbCodec
                    .encode(&fs_ipld)
                    .map_err(|e| CarError::Parsing(e.to_string()))?;
                let cid = pb_cid(&bs, hasher_codec);
                self.write_block(cid, &bs)?;
                Ok(cid)
            }
            _ => Err(CarError::Parsing("Not support write ipld.".to_lowercase())),
        }
    }

    fn rewrite_header(&mut self, header: CarHeader) -> Result<(), CarError>;

    fn flush(&mut self) -> Result<(), CarError>;
}

pub fn new_v1<W>(inner: W, header: CarHeader) -> Result<impl CarWriter, CarError>
where
    W: std::io::Write + std::io::Seek,
{
    Ok(CarWriterV1::new(inner, header))
}

pub fn new_v1_default_roots<W>(
    inner: W,
    hasher_codec: multicodec::Codec,
) -> Result<impl CarWriter, CarError>
where
    W: std::io::Write + std::io::Seek,
{
    Ok(CarWriterV1::new(
        inner,
        CarHeader::new_v1(vec![empty_pb_cid(hasher_codec)]),
    ))
}
