use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;

const INDEX_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RolloutSeekIndex {
    version: u32,
    compressed_size: u64,
    compressed_modified_ns: u128,
    pub(crate) uncompressed_size: u64,
    pub(crate) frames: Vec<FrameOffset>,
    pub(crate) checkpoints: Vec<CheckpointOffset>,
    pub(crate) session_meta_line: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct FrameOffset {
    pub(crate) uncompressed_offset: u64,
    pub(crate) compressed_offset: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct CheckpointOffset {
    pub(crate) uncompressed_offset: u64,
    pub(crate) line_checksum: u64,
    pub(crate) replacement_history: bool,
    pub(crate) window_number: bool,
    pub(crate) resume_metadata: bool,
}

pub(crate) fn line_checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

impl RolloutSeekIndex {
    pub(crate) fn new(uncompressed_size: u64) -> Self {
        Self {
            version: INDEX_VERSION,
            compressed_size: 0,
            compressed_modified_ns: 0,
            uncompressed_size,
            frames: Vec::new(),
            checkpoints: Vec::new(),
            session_meta_line: None,
        }
    }

    pub(crate) fn set_compressed_file(&mut self, path: &Path) -> io::Result<()> {
        let metadata = std::fs::metadata(path)?;
        self.compressed_size = metadata.len();
        self.compressed_modified_ns = modified_ns(metadata.modified()?);
        Ok(())
    }

    pub(crate) fn is_valid_for(&self, path: &Path) -> io::Result<bool> {
        let metadata = std::fs::metadata(path)?;
        Ok(self.version == INDEX_VERSION
            && metadata.len() == self.compressed_size
            && modified_ns(metadata.modified()?) == self.compressed_modified_ns
            && self.structure_is_valid())
    }

    fn structure_is_valid(&self) -> bool {
        let Some(first_frame) = self.frames.first() else {
            return false;
        };
        if first_frame.uncompressed_offset != 0 || first_frame.compressed_offset != 0 {
            return false;
        }
        let frames_are_ordered = self.frames.windows(2).all(|pair| {
            pair[0].uncompressed_offset < pair[1].uncompressed_offset
                && pair[0].compressed_offset < pair[1].compressed_offset
        });
        let checkpoints_are_ordered = self
            .checkpoints
            .windows(2)
            .all(|pair| pair[0].uncompressed_offset < pair[1].uncompressed_offset);
        frames_are_ordered
            && checkpoints_are_ordered
            && self.frames.iter().all(|frame| {
                frame.uncompressed_offset < self.uncompressed_size
                    && frame.compressed_offset < self.compressed_size
            })
            && self
                .checkpoints
                .iter()
                .all(|checkpoint| checkpoint.uncompressed_offset < self.uncompressed_size)
    }

    pub(crate) fn read_for(path: &Path) -> io::Result<Option<Self>> {
        let index_path = index_path(path);
        let index: Self = match std::fs::read(index_path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(index) => index,
                Err(_) => return Ok(None),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if index.is_valid_for(path)? {
            Ok(Some(index))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn write_for(&self, path: &Path) -> io::Result<()> {
        let index_path = index_path(path);
        let mut bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let parent = index_path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write as _;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        match std::fs::remove_file(&index_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        temp.persist_noclobber(&index_path)
            .map(|_| ())
            .map_err(|error| error.error)
    }
}

pub(crate) fn index_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".idx");
    PathBuf::from(name)
}

fn modified_ns(modified: SystemTime) -> u128 {
    modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
