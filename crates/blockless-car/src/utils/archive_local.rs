use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{self, Read, Seek},
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{
    codec::Encoder,
    error::CarError,
    header::CarHeaderV1,
    unixfs::{FileType, Link, UnixFs},
    writer::{CarWriter, CarWriterV1, WriteStream},
    CarHeader, Ipld,
};
use cid::{
    multihash::{Blake2b256, Code, Hasher, Multihash, MultihashDigest, Sha2_256},
    Cid,
};
use ipld::{pb::DagPbCodec, prelude::Codec, raw::RawCodec};
use path_absolutize::*;

type WalkPath = (Rc<PathBuf>, Option<usize>);
type WalkPathCache = HashMap<Rc<PathBuf>, UnixFs>;
type Size = usize;

const MAX_SECTION_SIZE: usize = 262144;

struct LimitedFile<'a> {
    inner: &'a mut File,
    readn: usize,
    limited: usize,
    pos: u64,
}

impl<'a> LimitedFile<'a> {
    fn new(inner: &'a mut File, limited: usize) -> Self {
        Self {
            pos: inner.stream_position().unwrap(),
            inner,
            limited,
            readn: 0,
        }
    }
}

impl<'a> Read for LimitedFile<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.readn == self.limited {
            return Ok(0);
        }
        let buf = if self.readn + buf.len() > self.limited {
            &mut buf[..(self.limited - self.readn)]
        } else {
            buf
        };
        self.inner.read(buf).map(|n| {
            self.readn += n;
            n
        })
    }
}

impl<'a> Seek for LimitedFile<'a> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match pos {
            io::SeekFrom::Start(0) => {
                self.inner.seek(io::SeekFrom::Start(self.pos))?;
                self.readn = 0;
                Ok(0)
            }
            _ => unimplemented!("Can only jump back to beginning of LimitedFile"),
        }
    }
}

trait HasherCodec {
    fn codec(&self) -> multicodec::Codec;
}

impl HasherCodec for Sha2_256 {
    fn codec(&self) -> multicodec::Codec {
        multicodec::Codec::Sha2_256
    }
}

impl HasherCodec for Blake2b256 {
    fn codec(&self) -> multicodec::Codec {
        multicodec::Codec::Blake2b_256
    }
}

fn cid_gen<H: Hasher + Default + HasherCodec>(
) -> impl FnMut(WriteStream) -> Option<Result<Cid, CarError>> {
    let mut hasher = H::default();
    move |w: WriteStream| match w {
        WriteStream::Bytes(bs) => {
            hasher.update(bs);
            None
        }
        WriteStream::End => {
            let code = hasher.codec();
            let bs = hasher.finalize();
            let h = match Multihash::wrap(code.code() as u64, bs) {
                Ok(h) => h,
                Err(e) => return Some(Err(CarError::Parsing(e.to_string()))),
            };
            Some(Ok(Cid::new_v1(RawCodec.into(), h)))
        }
    }
}

/// archive the directory to the target CAR format file
/// `path` is the directory archived in to the CAR file.
/// `to_carfile` is the target file.
pub fn archive_local<T>(
    path: impl AsRef<Path>,
    to_carfile: T,
    hasher_codec: multicodec::Codec,
    no_wrap_file: bool,
) -> Result<(), CarError>
where
    T: std::io::Write + std::io::Seek,
{
    let src_path = path.as_ref();
    if !src_path.exists() {
        return Err(CarError::IO(io::ErrorKind::NotFound.into()));
    }
    let root_path = src_path.absolutize().unwrap();
    let path = root_path.to_path_buf();
    // ensure sufficient file block size for head, after the root cid generated using the content, fill back the head.
    let mut root_cid = empty_pb_cid(hasher_codec);
    let header = CarHeader::new_v1(vec![root_cid]);
    let mut writer = CarWriterV1::new(to_carfile, header);

    if src_path.is_file() {
        // if the source is a file then do not walk directory tree, process the file directly
        let (hash, size) = process_file(src_path, &mut writer, hasher_codec)?;
        if no_wrap_file {
            root_cid = hash;
        } else {
            // wrap file into a directory entry
            let link = Link::new(
                hash,
                src_path.file_name().unwrap().to_str().unwrap().to_owned(),
                size as u64,
            );
            let unix_fs = UnixFs {
                links: vec![link],
                file_type: FileType::Directory,
                ..Default::default()
            };
            let dir_ipld = unix_fs.encode()?;
            let bs = DagPbCodec
                .encode(&dir_ipld)
                .map_err(|e| CarError::Parsing(e.to_string()))?;
            let cid = pb_cid(&bs, hasher_codec);
            writer.write_block(cid, bs)?;
            root_cid = cid;
        }
    } else {
        //source is a directory, walk the directory tree
        let (walk_paths, mut path_cache) = walk_path(path)?;
        for walk_path in &walk_paths {
            process_path(
                root_path.as_ref(),
                &mut root_cid,
                &mut writer,
                walk_path,
                &mut path_cache,
                hasher_codec,
            )?;
        }
    }
    let header = CarHeader::V1(CarHeaderV1::new(vec![root_cid]));
    writer.rewrite_header(header)
}

fn stream_block<R, W>(
    writer: &mut CarWriterV1<W>,
    stream_len: usize,
    r: &mut R,
    hasher_codec: multicodec::Codec,
) -> Result<Cid, CarError>
where
    W: std::io::Write + std::io::Seek,
    R: std::io::Read + std::io::Seek,
{
    match hasher_codec {
        multicodec::Codec::Sha2_256 => writer.stream_block(cid_gen::<Sha2_256>(), stream_len, r),
        multicodec::Codec::Blake2b_256 => {
            writer.stream_block(cid_gen::<Blake2b256>(), stream_len, r)
        }
        _ => unimplemented!(),
    }
}

fn process_file<W: std::io::Write + std::io::Seek>(
    path: &Path,
    writer: &mut CarWriterV1<W>,
    hasher_codec: multicodec::Codec,
) -> Result<(Cid, Size), CarError> {
    let mut file = fs::OpenOptions::new().read(true).open(path)?;
    let file_size = file.metadata()?.len() as usize;
    if file_size < MAX_SECTION_SIZE {
        Ok((
            stream_block(writer, file_size, &mut file, hasher_codec)?,
            file_size,
        ))
    } else {
        //split file when file size is bigger than the max section size.
        let file_secs = (file_size / MAX_SECTION_SIZE) + 1;
        //split the big file into small file and calc the cids.
        let mut block_sizes = vec![];
        let links = (0..file_secs)
            .map(|i| {
                let mut limit_file = LimitedFile::new(&mut file, MAX_SECTION_SIZE);
                let size = if i < file_secs - 1 {
                    MAX_SECTION_SIZE
                } else {
                    file_size % MAX_SECTION_SIZE
                };
                block_sizes.push(size as u64);
                let cid = stream_block(writer, size, &mut limit_file, hasher_codec);
                cid.map(|cid| Link::new(cid, String::new(), size as _))
            })
            .collect::<Result<Vec<Link>, CarError>>()?;
        let unix_fs_inner = UnixFs {
            links,
            file_type: FileType::File,
            file_size: Some(file_size as u64),
            block_sizes,
            ..Default::default()
        };
        let file_ipld = unix_fs_inner.encode()?;
        let bs = DagPbCodec
            .encode(&file_ipld)
            .map_err(|e| CarError::Parsing(e.to_string()))?;
        // add size of metadata to tsize https://discuss.ipfs.tech/t/how-to-decipher-root-node-content/11594/2
        let size = file_size + bs.len();
        let cid = pb_cid(&bs, hasher_codec);
        writer.write_block(cid, bs)?;
        Ok((cid, size))
    }
}

fn process_path<W: std::io::Write + std::io::Seek>(
    root_path: impl AsRef<Path>,
    root_cid: &mut Cid,
    writer: &mut CarWriterV1<W>,
    (abs_path, parent_idx): &(Rc<PathBuf>, Option<usize>),
    path_cache: &mut WalkPathCache,
    hasher_codec: multicodec::Codec,
) -> Result<(), CarError> {
    let unix_fs = path_cache.get_mut(abs_path).unwrap();
    let mut parent_tsize = 0;
    for link in unix_fs.links.iter_mut() {
        if let FileType::File = link.file_type {
            let (hash, size) = process_file(&abs_path.join(&link.name), writer, hasher_codec)?;
            link.hash = hash;
            link.tsize = size as u64;
        }
        parent_tsize += link.tsize;
    }
    // sort links correctly for pb-dag standard https://ipld.io/specs/codecs/dag-pb/spec/#link-sorting
    unix_fs
        .links
        .sort_by(|a, b| match a.name.as_bytes() > b.name.as_bytes() {
            true => std::cmp::Ordering::Greater,
            false => std::cmp::Ordering::Less,
        });

    let fs_ipld: Ipld = unix_fs.encode()?;
    let bs = DagPbCodec
        .encode(&fs_ipld)
        .map_err(|e| CarError::Parsing(e.to_string()))?;
    parent_tsize += bs.len() as u64;
    let cid = pb_cid(&bs, hasher_codec);
    if root_path.as_ref() == abs_path.as_ref() {
        *root_cid = cid;
    }
    writer.write_block(cid, bs)?;
    unix_fs.cid = Some(cid);
    match abs_path.parent() {
        Some(parent) => {
            let parent = Rc::new(parent.to_path_buf());
            if let Some((p, pos)) = path_cache.get_mut(&parent).zip(*parent_idx) {
                p.links[pos].hash = cid;
                p.links[pos].tsize = parent_tsize;
            }
        }
        None => unimplemented!("should not happen"),
    }
    Ok(())
}

fn digest(data: &[u8], hasher_codec: multicodec::Codec) -> Multihash {
    match hasher_codec {
        multicodec::Codec::Sha2_256 => Code::Sha2_256.digest(data),
        multicodec::Codec::Blake2b_256 => Code::Blake2b256.digest(data),
        _ => unimplemented!(),
    }
}

#[inline(always)]
pub fn empty_pb_cid(hasher_codec: multicodec::Codec) -> Cid {
    pb_cid(&[], hasher_codec)
}

#[inline(always)]
pub fn pb_cid(data: &[u8], hasher_codec: multicodec::Codec) -> Cid {
    Cid::new_v1(DagPbCodec.into(), digest(data, hasher_codec))
}

#[inline(always)]
pub fn raw_cid(data: &[u8], hasher_codec: multicodec::Codec) -> Cid {
    Cid::new_v1(RawCodec.into(), digest(data, hasher_codec))
}

/// walk all directory, and record the directory informations.
/// `WalkPath` contain the index in children.
pub fn walk_path(path: impl AsRef<Path>) -> Result<(Vec<WalkPath>, WalkPathCache), CarError> {
    let root_path: Rc<PathBuf> = Rc::new(path.as_ref().absolutize()?.into());
    let mut queue = VecDeque::from(vec![root_path.clone()]);
    let mut path_cache = HashMap::new();
    let mut walk_paths = Vec::new();
    while let Some(dir_path) = queue.pop_back() {
        // let file_type = fs::metadata(root_path.as_ref())?.file_type();
        let mut unix_dir = UnixFs::new_directory();
        // if file_type.is_file() {
        //     // no_wrap true
        //     unix_dir.file_type = FileType::File;
        //     let name = dir_path
        //         .file_name()
        //         .unwrap_or_default()
        //         .to_str()
        //         .unwrap()
        //         .to_string();
        //     let tsize = fs::metadata(root_path.as_ref())?.len();
        //     unix_dir.add_link(Link {
        //         name,
        //         tsize,
        //         file_type: FileType::File,
        //         ..Default::default()
        //     });
        // } else if file_type.is_dir() {
        // no_wrap false
        // unix_dir.file_type = FileType::Directory;
        for entry in fs::read_dir(&*dir_path)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name().to_str().unwrap_or("").to_string();
            if file_type.is_file() {
                unix_dir.add_link(Link {
                    name,
                    file_type: FileType::File,
                    ..Default::default()
                });
            } else if file_type.is_dir() {
                // sum up all file sizes in this dir for tsize in pb-dag
                // let mut dir_size = 0;
                // for entry in fs::read_dir(entry.path())? {
                //     let entry = entry?;
                //     if entry.file_type()?.is_file() {
                //         dir_size+= entry.metadata()?.len();
                //     }
                // }
                let abs_path = entry.path().absolutize()?.to_path_buf();
                let rc_abs_path = Rc::new(abs_path);
                let idx = unix_dir.add_link(Link {
                    name,
                    tsize: 0,
                    file_type: FileType::Directory,
                    ..Default::default()
                });
                walk_paths.push((rc_abs_path.clone(), Some(idx)));
                queue.push_back(rc_abs_path);
            }
            // }
            // } else {
            //     unreachable!("unsupported filetype!");
        }
        path_cache.insert(dir_path, unix_dir);
    }

    walk_paths.reverse();
    walk_paths.push((root_path, None));

    Ok((walk_paths, path_cache))
}

#[cfg(test)]
mod test {
    use super::*;
    use hex::ToHex;
    use std::io::Write;
    use tempdir::TempDir;

    #[test]
    fn test_archive_local_dir_nested() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "22dcd3f17a1abd64c785fc796ce6593ce4a501717aa6352f3f3c11973d240f96";

        let temp_dir = TempDir::new("blockless-car-temp-dir-1").unwrap();
        let temp_dir_nested = temp_dir.path().join("nested");
        std::fs::create_dir_all(temp_dir_nested.as_ref() as &Path).unwrap();

        let temp_file = temp_dir_nested.join("test.txt");
        let mut file = File::create(temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        archive_local(&temp_dir, &car_file, multicodec::Codec::Sha2_256, false).unwrap();

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }

    #[test]
    fn test_archive_local_small_file_no_wrap_false() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "81a61aa2d2c34f128720b8639e39503b30ffe7facbba5454de125649c41887b8";

        let temp_dir = TempDir::new("blockless-car-temp-dir-2").unwrap();
        let temp_file = temp_dir.path().join("test.txt");

        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the file
        archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, false).unwrap();

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(&temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }

    #[test]
    fn test_archive_local_small_file_no_wrap_true() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "7749e28c4fe3f68c00ac08af41c1c4f6e0275c86bd9e8ae7b9446da7d1663710";

        let temp_dir = TempDir::new("blockless-car-temp-dir-3").unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the directory with no-wrap
        assert!(archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, true).is_ok());

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(&temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }

    #[test]
    fn test_archive_local_large_file_no_wrap_false() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "561d2686a9d062095cc6be8e997b554b244fac4380daa254444fc5b7195dfb37";

        let temp_dir = TempDir::new("blockless-car-temp-dir-2").unwrap();
        let temp_file = temp_dir.path().join("data.bin");
        std::process::Command::new("sh")
            .arg("-c")
            .arg("dd if=/dev/zero bs=1000000 count=1 > ".to_string() + temp_file.to_str().unwrap())
            .output()
            .expect("failed to execute process");

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the file
        archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, false).unwrap();

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(&temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }

    #[test]
    fn test_archive_local_large_file_no_wrap_true() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "aa0ff19fc2c63eb6ec5a53eebc471b06187a48ba0e19f2e494671171c5eb6279";

        let temp_dir = TempDir::new("blockless-car-temp-dir-3").unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        std::process::Command::new("sh")
            .arg("-c")
            .arg("dd if=/dev/zero bs=1000000 count=1 > ".to_string() + temp_file.to_str().unwrap())
            .output()
            .expect("failed to execute process");

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the directory with no-wrap
        assert!(archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, true).is_ok());

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(&temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }

    #[test]
    fn test_archive_local_dir_small_file() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "81a61aa2d2c34f128720b8639e39503b30ffe7facbba5454de125649c41887b8";

        let temp_dir = TempDir::new("blockless-car-temp-dir-3").unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        let mut file = File::create(temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the directory with no-wrap
        assert!(archive_local(&temp_dir, &car_file, multicodec::Codec::Sha2_256, false).is_ok());

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }

    #[test]
    fn test_archive_local_dir_big_file() {
        // SHA2-256 hash of a reference .car file generated with go-car (https://github.com/ipld/go-car)
        const REFERENCE_HASH: &str =
            "561d2686a9d062095cc6be8e997b554b244fac4380daa254444fc5b7195dfb37";

        let temp_dir = TempDir::new("blockless-car-temp-dir-3").unwrap();
        let temp_file = temp_dir.path().join("data.bin");
        std::process::Command::new("sh")
            .arg("-c")
            .arg("dd if=/dev/zero bs=1000000 count=1 > ".to_string() + temp_file.to_str().unwrap())
            .output()
            .expect("failed to execute process");

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the directory with no-wrap
        assert!(archive_local(&temp_dir, &car_file, multicodec::Codec::Sha2_256, false).is_ok());

        // compare hash of CAR file to precomputed hash
        let bytes = std::fs::read(temp_output_file).unwrap(); // Vec<u8>
        let hash = Code::Sha2_256.digest(&bytes);
        assert_eq!(hash.digest().encode_hex::<String>(), REFERENCE_HASH);
    }
}
