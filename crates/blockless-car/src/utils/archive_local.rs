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
const MAX_LINK_COUNT: usize = 174;

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
) -> Result<Cid, CarError>
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
            let link = Link {
                hash,
                file_type: FileType::Directory,
                name: src_path.file_name().unwrap().to_str().unwrap().to_owned(),
                tsize: size as u64,
            };
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
        let (walk_paths, mut path_cache) = walk_path(&path)?;
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
        // add an additional top node like in go-car
        let root_node = path_cache.get(&path).unwrap();
        let tsize: u64 = DagPbCodec
            .encode(&root_node.encode()?)
            .map_err(|e| CarError::Parsing(e.to_string()))?
            .len() as u64
            + root_node.links.iter().map(|link| link.tsize).sum::<u64>();
        let unix_fs = UnixFs {
            links: vec![Link {
                hash: root_cid,
                file_type: FileType::Directory,
                name: path.file_name().unwrap().to_str().unwrap().to_string(),
                tsize,
            }],
            file_type: FileType::Directory,
            file_size: None,
            ..Default::default()
        };
        let ipld = unix_fs.encode()?;
        let bs = DagPbCodec
            .encode(&ipld)
            .map_err(|e| CarError::Parsing(e.to_string()))?;
        root_cid = pb_cid(&bs, hasher_codec);
        writer.write_block(root_cid, bs)?;
    }
    let header = CarHeader::V1(CarHeaderV1::new(vec![root_cid]));
    writer.rewrite_header(header)?;
    Ok(root_cid)
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
        let mut secs = file_size / MAX_SECTION_SIZE;
        if file_size % MAX_SECTION_SIZE > 0 {
            secs += 1;
        }
        let mut block_sizes = vec![];
        let mut links = (0..secs)
            .map(|i| {
                let mut limit_file = LimitedFile::new(&mut file, MAX_SECTION_SIZE);
                let size = if i < secs - 1 {
                    MAX_SECTION_SIZE
                } else {
                    file_size % MAX_SECTION_SIZE
                };
                block_sizes.push(size as u64);
                let cid = stream_block(writer, size, &mut limit_file, hasher_codec);
                cid.map(|cid| Link {
                    hash: cid,
                    file_type: FileType::Raw,
                    name: String::default(),
                    tsize: size as u64,
                })
            })
            .collect::<Result<Vec<Link>, CarError>>()?;
        while links.len() > MAX_LINK_COUNT {
            let mut new_links = vec![];
            let mut new_block_sizes = vec![];
            let mut link_count = links.len() / MAX_LINK_COUNT;
            if links.len() % MAX_LINK_COUNT > 0 {
                link_count += 1;
            }
            for _ in 0..link_count {
                let len = if links.len() >= MAX_LINK_COUNT {
                    MAX_LINK_COUNT
                } else {
                    links.len()
                };
                let links_size = block_sizes.as_slice()[0..len].iter().sum();
                let unix_fs = UnixFs {
                    links: links.drain(0..len).collect(),
                    file_type: FileType::File,
                    file_size: Some(links_size),
                    block_sizes: block_sizes.drain(0..len).collect(),
                    ..Default::default()
                };
                let ipld = unix_fs.encode()?;
                let bs = DagPbCodec
                    .encode(&ipld)
                    .map_err(|e| CarError::Parsing(e.to_string()))?;
                let size = links_size + bs.len() as u64;
                let cid = pb_cid(&bs, hasher_codec);
                writer.write_block(cid, bs)?;
                let new_link = Link {
                    hash: cid,
                    file_type: FileType::File,
                    name: String::default(),
                    tsize: size,
                };
                new_links.push(new_link);
                new_block_sizes.push(links_size);
            }
            links = new_links;
            block_sizes = new_block_sizes;
        }
        let links_size = links.iter().map(|link| link.tsize as usize).sum::<usize>();
        let unix_fs = UnixFs {
            file_size: Some(block_sizes.iter().sum()),
            links,
            file_type: FileType::File,
            block_sizes,
            ..Default::default()
        };
        let file_ipld = unix_fs.encode()?;
        let bs = DagPbCodec
            .encode(&file_ipld)
            .map_err(|e| CarError::Parsing(e.to_string()))?;
        let size = links_size + bs.len();
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
        let mut unix_dir = UnixFs::new_directory();
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
    use rand::prelude::*;
    use rand_chacha::ChaCha8Rng;
    use std::{
        cmp,
        io::{BufWriter, Write},
        str::FromStr,
    };
    use tempdir::TempDir;

    fn write_large_file(path: &PathBuf, size: usize) {
        let file = File::create(path).unwrap();
        let mut writer = BufWriter::new(file);
        let mut buffer: [u8; 1000] = [0; 1000];
        let mut remaining_size = size;
        // use seeded random data to fill
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        while remaining_size > 0 {
            let to_write = cmp::min(remaining_size, buffer.len());
            let buffer = &mut buffer[..to_write];
            rng.fill(buffer);
            let amount = writer.write(buffer).unwrap();
            remaining_size -= amount;
        }
        writer.flush().unwrap();
    }

    fn get_reference_cid(
        source_path: &impl AsRef<Path>,
        output_dir: &impl AsRef<Path>,
        no_wrap: bool,
    ) -> Option<Cid> {
        if !home::home_dir().unwrap().join("go/bin/car").exists() {
            return None;
        }
        let temp_reference_file = output_dir.as_ref().join("test-reference.car");
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "$HOME/go/bin/car create --version 1 {} --file {} {}",
                if no_wrap { "--no-wrap" } else { "" },
                temp_reference_file.to_str().unwrap(),
                source_path.as_ref().to_str().unwrap()
            ))
            .output()
            .expect("failed to execute process");
        let result = String::from_utf8(
            std::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "$HOME/go/bin/car root {}",
                    temp_reference_file.to_str().unwrap(),
                ))
                .output()
                .expect("failed to execute process")
                .stdout,
        )
        .unwrap();
        let reference = Cid::from_str(result.trim()).unwrap();
        println!("Reference CID: {}", reference);
        Some(reference)
    }

    #[test]
    fn test_small_file_no_wrap_false() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();

        // create a root dir with a fixed name (temp_dir name has a random suffix)
        let root_dir = temp_dir.path().join("root");
        std::fs::create_dir_all(root_dir).unwrap();

        let temp_file = temp_dir.path().join("test.txt");

        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&temp_file, &temp_output_dir, false) {
            Some(reference) => reference,
            None => Cid::from_str("bafybeifotw2dmp73obnbhg6uffdrjshvone2jkkp3rlw3fot2vne5zvymu")
                .unwrap(),
        };

        let test_cid =
            archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, false).unwrap();
        assert_eq!(test_cid, reference);
    }

    #[test]
    fn test_small_file_no_wrap_true() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();
        let temp_file = temp_dir.path().join("test.txt");
        let mut file = File::create(&temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&temp_file, &temp_output_dir, true) {
            Some(reference) => reference,
            None => Cid::from_str("bafkreifzjut3te2nhyekklss27nh3k72ysco7y32koao5eei66wof36n5e")
                .unwrap(),
        };

        let test_cid =
            archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, true).unwrap();
        assert_eq!(test_cid, reference);
    }

    #[test]
    fn test_large_file_no_wrap_false() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();

        // create a root dir with a fixed name (temp_dir name has a random suffix)
        let root_dir = temp_dir.path().join("root");
        std::fs::create_dir_all(root_dir).unwrap();

        let temp_file = temp_dir.path().join("data.bin");
        write_large_file(&temp_file, 1000000);

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&temp_file, &temp_output_dir, false) {
            Some(reference) => reference,
            None => Cid::from_str("bafybeibdndwligqskbbklvjhq32fuugwfuzt3i242u2yd2ih6hddgmilkm")
                .unwrap(),
        };

        let test_cid =
            archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, false).unwrap();
        assert_eq!(test_cid, reference);
    }

    #[test]
    fn test_large_file_no_wrap_true() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();
        let temp_file = temp_dir.path().join("data.bin");
        write_large_file(&temp_file, 1000000);

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&temp_file, &temp_output_dir, true) {
            Some(reference) => reference,
            None => Cid::from_str("bafybeigr5o3jbe2biam6pskvjhbaczjfdlmnjwlzovpgbzctiwqtpkvhee")
                .unwrap(),
        };

        let test_cid =
            archive_local(&temp_file, &car_file, multicodec::Codec::Sha2_256, true).unwrap();
        assert_eq!(test_cid, reference);
    }

    #[test]
    fn test_dir_small_file() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();

        // create a root dir with a fixed name (temp_dir name has a random suffix)
        let root_dir = temp_dir.path().join("root");
        std::fs::create_dir_all(&root_dir).unwrap();

        let temp_file = temp_dir.path().join("test.txt");
        let mut file = File::create(temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&root_dir, &temp_output_dir, false) {
            Some(reference) => reference,
            None => Cid::from_str("bafybeifp6fbcoaq3px3ha22ddltu3itl5ek3secgtmbwm4ui7ru74ndwkm")
                .unwrap(),
        };

        let test_cid =
            archive_local(&root_dir, &car_file, multicodec::Codec::Sha2_256, false).unwrap();
        assert_eq!(test_cid, reference);
    }

    #[test]
    fn test_dir_big_file() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();

        // create a root dir with a fixed name (temp_dir name has a random suffix)
        let root_dir = temp_dir.path().join("root");
        std::fs::create_dir_all(&root_dir).unwrap();

        let temp_file = root_dir.join("data.bin");
        write_large_file(&temp_file, 1000000000);

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&root_dir, &temp_output_dir, false) {
            Some(reference) => reference,
            None => Cid::from_str("bafybeidvyeyyss53sab3i43utmznutnise2h7ptvv3ftccvyfqc6r5sv74")
                .unwrap(),
        };

        let test_cid =
            archive_local(&root_dir, &car_file, multicodec::Codec::Sha2_256, false).unwrap();
        assert_eq!(test_cid, reference);
    }

    #[test]
    fn dir_tree() {
        let temp_dir = TempDir::new("blockless-car-temp-dir").unwrap();

        // create a root dir with a fixed name (temp_dir name has a random suffix)
        let root_dir = temp_dir.path().join("root");

        std::fs::create_dir_all(root_dir.join("level1A/level2A/level3A")).unwrap();
        std::fs::create_dir_all(root_dir.join("level1A/level2B/level3A")).unwrap();
        std::fs::create_dir_all(root_dir.join("level1A/level2C/level3A")).unwrap();
        std::fs::create_dir_all(root_dir.join("level1B/level2A/level3A")).unwrap();

        let temp_file = temp_dir
            .path()
            .join("root/level1A/level2A/level3A/test.txt");
        let mut file = File::create(temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_file = root_dir.join("level1A/level2A/test.txt");
        let mut file = File::create(temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_file = temp_dir
            .path()
            .join("root/level1A/level2B/level3A/data.bin");
        write_large_file(&temp_file, 1000000);

        let temp_file = temp_dir
            .path()
            .join("root/level1A/level2C/level3A/data.bin");
        write_large_file(&temp_file, 100000000);

        let temp_file = temp_dir
            .path()
            .join("root/level1A/level2C/level3A/test.txt");
        let mut file = File::create(temp_file).unwrap();
        file.write_all(b"hello world").unwrap();

        let temp_output_dir = TempDir::new("blockless-car-temp-output-dir").unwrap();
        let temp_output_file = temp_output_dir.path().join("test.car");
        let car_file = std::fs::File::create(temp_output_file.as_ref() as &Path).unwrap();

        let reference = match get_reference_cid(&root_dir, &temp_output_dir, false) {
            Some(reference) => reference,
            None => Cid::from_str("bafybeicidmis4mrywfe4almb473raq7upvacl2hk6lxqsi2zggvrj7demi")
                .unwrap(),
        };

        let test_cid =
            archive_local(&root_dir, &car_file, multicodec::Codec::Sha2_256, false).unwrap();
        assert_eq!(test_cid, reference);
    }
}
