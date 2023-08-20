use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{self, Read},
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
    multihash::{Blake2b256, Code, Hasher, Multihash, MultihashDigest},
    Cid,
};
use ipld::{pb::DagPbCodec, prelude::Codec, raw::RawCodec};
use path_absolutize::*;

use super::BLAKE2B256_CODEC;

type WalkPath = (Rc<PathBuf>, Option<usize>);

type WalkPathCache = HashMap<Rc<PathBuf>, UnixFs>;

const MAX_SECTION_SIZE: usize = 8 << 20;

struct LimitedFile<'a> {
    inner: &'a mut File,
    readn: usize,
    limited: usize,
}

impl<'a> LimitedFile<'a> {
    fn new(inner: &'a mut File, limited: usize) -> Self {
        Self {
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

fn cid_gen() -> impl FnMut(WriteStream) -> Option<Result<Cid, CarError>> {
    let mut hash_codec = Blake2b256::default();
    move |w: WriteStream| match w {
        WriteStream::Bytes(bs) => {
            hash_codec.update(bs);
            None
        }
        WriteStream::End => {
            let bs = hash_codec.finalize();
            let h = match Multihash::wrap(BLAKE2B256_CODEC, bs) {
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
    no_wrap_file: bool,
) -> Result<(), CarError>
where
    T: std::io::Write + std::io::Seek,
{
    let mut src_path = path.as_ref();
    if !src_path.exists() {
        return Err(CarError::IO(io::ErrorKind::NotFound.into()));
    }
    if src_path.is_file() && !no_wrap_file {
        // no-wrap only applicable to files
        src_path = src_path.parent().unwrap();
    }
    let root_path = src_path.absolutize().unwrap();
    let path = root_path.to_path_buf();
    // ensure sufficient file block size for head, after the root cid generated using the content, fill back the head.
    let mut root_cid = Some(empty_pb_cid());
    let header = CarHeader::new_v1(vec![root_cid.unwrap()]);
    let mut writer = CarWriterV1::new(to_carfile, header);

    let (walk_paths, mut path_cache) = walk_path(path)?;
    for walk_path in &walk_paths {
        process_path(
            root_path.as_ref(),
            &mut root_cid,
            &mut writer,
            walk_path,
            &mut path_cache,
        )?;
    }

    let root_cid = root_cid.ok_or(CarError::NotFound("root cid not found.".to_string()))?;
    let header = CarHeader::V1(CarHeaderV1::new(vec![root_cid]));
    writer.rewrite_header(header)
}

fn process_path<W: std::io::Write + std::io::Seek>(
    root_path: impl AsRef<Path>,
    root_cid: &mut Option<Cid>,
    writer: &mut CarWriterV1<W>,
    (abs_path, parent_idx): &(Rc<PathBuf>, Option<usize>),
    path_cache: &mut WalkPathCache,
) -> Result<(), CarError> {
    let unixfs = path_cache.get_mut(abs_path).unwrap();
    let file_type = unixfs.file_type();
    for link in unixfs.links.iter_mut() {
        match link.guess_type {
            FileType::Directory => {} // ignore processing directory
            FileType::File => {
                let filepath: PathBuf = match file_type {
                    FileType::File => abs_path.to_path_buf(),
                    FileType::Directory => abs_path.join(link.name_ref()),
                    _ => unreachable!("invalid file type"),
                };
                let mut file = fs::OpenOptions::new().read(true).open(filepath)?;
                let file_size = link.tsize as usize;
                if file_size < MAX_SECTION_SIZE {
                    let file_cid = writer.write_stream(cid_gen(), file_size, &mut file)?;
                    link.hash = file_cid;
                } else {
                    //split file when file size is bigger than the max section size.
                    let file_secs = (file_size / MAX_SECTION_SIZE) + 1;
                    //split the big file into small file and calc the cids.
                    let links = (0..file_secs)
                        .map(|i| {
                            let mut limit_file = LimitedFile::new(&mut file, MAX_SECTION_SIZE);
                            let size = if i < file_secs - 1 {
                                MAX_SECTION_SIZE
                            } else {
                                file_size % MAX_SECTION_SIZE
                            };
                            let cid = writer.write_stream(cid_gen(), size, &mut limit_file);
                            cid.map(|cid| Link::new(cid, String::new(), size as _))
                        })
                        .collect::<Result<Vec<Link>, CarError>>()?;
                    let unix_fs = UnixFs {
                        links,
                        file_type: FileType::File,
                        file_size: Some(file_size as u64),
                        ..Default::default()
                    };
                    let file_ipld = unix_fs.encode()?;
                    let bs = DagPbCodec
                        .encode(&file_ipld)
                        .map_err(|e| CarError::Parsing(e.to_string()))?;
                    let cid = pb_cid(&bs);
                    writer.write(cid, bs)?;
                    link.hash = cid;
                }
            }
            _ => unreachable!("unsupported filetype!"),
        }
    }
    let fs_ipld: Ipld = unixfs.encode()?;
    let bs = DagPbCodec
        .encode(&fs_ipld)
        .map_err(|e| CarError::Parsing(e.to_string()))?;
    let cid = pb_cid(&bs);
    if root_path.as_ref() == abs_path.as_ref() {
        *root_cid = Some(cid);
    }
    writer.write(cid, bs)?;
    unixfs.cid = Some(cid);
    match abs_path.parent() {
        Some(parent) => {
            let parent = Rc::new(parent.to_path_buf());

            if let Some((p, pos)) = path_cache.get_mut(&parent).zip(*parent_idx) {
                p.links[pos].hash = cid;
            }
        }
        None => unimplemented!("should not happen"),
    }
    Ok(())
}

pub fn pipe_raw_cid<R, W>(r: &mut R, w: &mut W) -> Result<Cid, CarError>
where
    R: std::io::Read,
    W: std::io::Write,
{
    let mut hash_codec = cid::multihash::Blake2b256::default();
    let mut bs = [0u8; 1024];
    while let Ok(n) = r.read(&mut bs) {
        hash_codec.update(&bs[0..n]);
        w.write_all(&bs[0..n])?;
    }
    let bs = hash_codec.finalize();
    let h = cid::multihash::Multihash::wrap(BLAKE2B256_CODEC, bs);
    let h = h.map_err(|e| CarError::Parsing(e.to_string()))?;
    Ok(Cid::new_v1(DagPbCodec.into(), h))
}

#[inline(always)]
pub fn empty_pb_cid() -> Cid {
    pb_cid(&[])
}

#[inline(always)]
pub fn pb_cid(data: &[u8]) -> Cid {
    let h = Code::Blake2b256.digest(data);
    Cid::new_v1(DagPbCodec.into(), h)
}

#[inline(always)]
pub fn raw_cid(data: &[u8]) -> Cid {
    let h = Code::Blake2b256.digest(data);
    Cid::new_v1(RawCodec.into(), h)
}

/// walk all directory, and record the directory informations.
/// `WalkPath` contain the index in children.
pub fn walk_path(path: impl AsRef<Path>) -> Result<(Vec<WalkPath>, WalkPathCache), CarError> {
    let root_path: Rc<PathBuf> = Rc::new(path.as_ref().absolutize()?.into());

    let mut queue = VecDeque::from(vec![root_path.clone()]);
    let mut path_cache = HashMap::new();
    let mut walk_paths = Vec::new();
    while let Some(dir_path) = queue.pop_back() {
        let file_type = fs::metadata(root_path.as_ref())?.file_type();
        let mut unix_dir = UnixFs::default();
        if file_type.is_file() {
            unix_dir.file_type = FileType::File;
            let name = dir_path
                .file_name()
                .unwrap_or_default()
                .to_str()
                .unwrap()
                .to_string();
            let tsize = fs::metadata(root_path.as_ref())?.len();
            unix_dir.add_link(Link {
                name,
                tsize,
                guess_type: FileType::File,
                ..Default::default()
            });
        } else if file_type.is_dir() {
            unix_dir.file_type = FileType::Directory;
            for entry in fs::read_dir(&*dir_path)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                let name = entry.file_name().to_str().unwrap_or("").to_string();
                let tsize = entry.metadata()?.len();

                if file_type.is_file() {
                    unix_dir.add_link(Link {
                        name,
                        tsize,
                        guess_type: FileType::File,
                        ..Default::default()
                    });
                } else if file_type.is_dir() {
                    let abs_path = entry.path().absolutize()?.to_path_buf();
                    let rc_abs_path = Rc::new(abs_path);
                    let idx = unix_dir.add_link(Link {
                        name,
                        tsize,
                        guess_type: FileType::Directory,
                        ..Default::default()
                    });
                    walk_paths.push((rc_abs_path.clone(), Some(idx)));
                    queue.push_back(rc_abs_path);
                }
            }
        } else {
            unreachable!("unsupported filetype!");
        };
        path_cache.insert(dir_path, unix_dir);
    }

    walk_paths.reverse();
    walk_paths.push((root_path, None));

    Ok((walk_paths, path_cache))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;
    use tempdir::TempDir;

    #[test]
    fn test_archive_local_dir_nested() {
        // create a temp file: /tmp/blockless-car-temp-dir-1/nested/test.txt
        let temp_dir = TempDir::new("blockless-car-temp-dir-1").unwrap();
        let temp_dir_nested = temp_dir.path().join("nested");
        std::fs::create_dir_all(temp_dir_nested.as_ref() as &Path).unwrap();

        let temp_file = temp_dir_nested.join("test.txt");
        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        archive_local(&temp_dir, &car_file, false).unwrap();

        assert_eq!(car_file.metadata().unwrap().len(), 303);
    }

    #[test]
    fn test_archive_local_file_no_wrap() {
        // create a temp file: /tmp/blockless-car-temp-dir-2/test.txt
        let temp_dir = TempDir::new("blockless-car-temp-dir-2").unwrap();
        let temp_file = temp_dir.path().join("test.txt");

        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the file
        archive_local(&temp_file, &car_file, false).unwrap();

        assert_eq!(car_file.metadata().unwrap().len(), 208);

        let temp_output_file = temp_output_dir.path().join("test-dir.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the dir (parent of the file)
        archive_local(&temp_dir, &car_file, false).unwrap();

        assert_eq!(car_file.metadata().unwrap().len(), 208); // same as the file
    }

    #[test]
    fn test_file_wrapping_supported() {
        // create a temp file: /tmp/blockless-car-temp-dir-3/test.txt
        let temp_dir = TempDir::new("blockless-car-temp-dir-3").unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        // archive the file with no-wrap
        assert_eq!(archive_local(&temp_dir, &car_file, true).is_err(), false);

        // validate car-file is created and has content
        assert_eq!(car_file.metadata().unwrap().len(), 208);
    }
}
