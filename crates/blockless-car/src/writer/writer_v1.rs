use super::{CarWriter, WriteStream};
use crate::{error::CarError, header::CarHeader};
use cid::Cid;
use integer_encoding::VarIntWriter;

// how many bytes to read at once from stream
const BUFFER_SIZE: usize = 10240;

pub(crate) struct CarWriterV1<W> {
    inner: W,
    header: CarHeader,
    is_header_written: bool,
    hashes_written: Vec<Cid>,
}

impl<W> CarWriterV1<W>
where
    W: std::io::Write + std::io::Seek,
{
    fn write_head(&mut self) -> Result<(), CarError> {
        let head = self.header.encode()?;
        self.inner.write_varint(head.len())?;
        self.inner.write_all(&head)?;
        self.is_header_written = true;
        Ok(())
    }

    pub fn new(inner: W, header: CarHeader) -> Self {
        Self {
            inner,
            header,
            is_header_written: false,
            hashes_written: vec![],
        }
    }
}

impl<W> CarWriter for CarWriterV1<W>
where
    W: std::io::Write + std::io::Seek,
{
    fn write_block<T>(&mut self, cid: cid::Cid, data: T) -> Result<(), CarError>
    where
        T: AsRef<[u8]>,
    {
        if !self.is_header_written {
            self.write_head()?;
        }
        if !self.hashes_written.contains(&cid) {
            let mut cid_buff: Vec<u8> = Vec::new();
            cid.write_bytes(&mut cid_buff)
                .map_err(|e| CarError::Parsing(e.to_string()))?;
            let data = data.as_ref();
            let sec_len = data.len() + cid_buff.len();
            self.inner.write_varint(sec_len)?;
            self.inner.write_all(&cid_buff[..])?;
            self.inner.write_all(data)?;
            self.hashes_written.push(cid);
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CarError> {
        self.inner.flush()?;
        Ok(())
    }

    fn rewrite_header(&mut self, header: CarHeader) -> Result<(), CarError> {
        if header.roots().len() != self.header.roots().len() {
            return Err(CarError::InvalidSection(
                "the root cid is not match.".to_string(),
            ));
        }
        self.header = header;
        self.inner.rewind()?;
        self.write_head()
    }

    fn stream_block<F, R>(
        &mut self,
        mut cid_f: F,
        stream_size: usize,
        r: &mut R,
    ) -> Result<Cid, CarError>
    where
        R: std::io::Read + std::io::Seek,
        F: FnMut(WriteStream) -> Option<Result<Cid, CarError>>,
    {
        if !self.is_header_written {
            self.write_head()?;
        }
        let mut read_size = 0;

        // store start position in stream
        let start_pos = r.stream_position()?;

        // stream r once to get CID
        let mut buffer = [0u8; BUFFER_SIZE];
        while let Ok(n) =
            r.read(&mut buffer[0..std::cmp::min(BUFFER_SIZE, stream_size - read_size)])
        {
            if n == 0 {
                break;
            }
            read_size += n;
            if let Some(Err(e)) = cid_f(WriteStream::Bytes(&buffer[0..n])) {
                return Err(e);
            }
        }
        let cid = match cid_f(WriteStream::End) {
            Some(Ok(cid)) => cid,
            Some(Err(e)) => return Err(e),
            None => unreachable!("cid function cannot return None here"),
        };

        if !self.hashes_written.contains(&cid) {
            // write length and CID to stream
            let mut cid_buf: Vec<u8> = Vec::new();
            cid.write_bytes(&mut cid_buf)
                .map_err(|e| CarError::Parsing(e.to_string()))?;
            let sec_len = stream_size + cid_buf.len();
            self.inner.write_varint(sec_len)?;
            self.inner.write_all(cid_buf.as_slice())?;

            // stream r a second time to write into output stream
            let mut read_size = 0;
            r.seek(std::io::SeekFrom::Start(start_pos))?;
            while let Ok(n) =
                r.read(&mut buffer[0..std::cmp::min(BUFFER_SIZE, stream_size - read_size)])
            {
                if n == 0 {
                    break;
                }
                read_size += n;
                self.inner.write_all(&buffer[0..n])?;
            }
            self.hashes_written.push(cid);
        }
        Ok(cid)
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;

    use ipld_cbor::DagCborCodec;

    use crate::header::{CarHeader, CarHeaderV1};
    use crate::reader::{CarReader, CarReaderV1};

    use super::*;
    use cid::multihash::{Code::Blake2b256, MultihashDigest};
    use cid::Cid;

    #[test]
    fn test_writer_read_v1() {
        let digest_test = Blake2b256.digest(b"test");
        let cid_test1 = Cid::new_v1(DagCborCodec.into(), digest_test);
        let digest_test2 = Blake2b256.digest(b"test2");
        let cid_test2 = Cid::new_v1(DagCborCodec.into(), digest_test2);
        let header = CarHeader::V1(CarHeaderV1::new(vec![cid_test2]));
        let mut buffer = Vec::new();
        let mut buf = Cursor::new(&mut buffer);
        let mut writer = CarWriterV1::new(&mut buf, header);
        writer.write_block(cid_test1, b"test1").unwrap();
        writer.write_block(cid_test2, b"test2").unwrap();
        writer.flush().unwrap();
        let mut reader = Cursor::new(&buffer);
        let car_reader = CarReaderV1::new(&mut reader).unwrap();
        assert_eq!(vec![cid_test2], car_reader.header().roots());
        assert_eq!(car_reader.sections().len(), 2);
    }
}
