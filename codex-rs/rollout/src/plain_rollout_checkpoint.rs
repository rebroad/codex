use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

const INDEX_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct PlainRolloutCheckpoint {
    version: u32,
    pub(crate) offset: u64,
}

impl PlainRolloutCheckpoint {
    pub(crate) fn read_for(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(index_path(path)).ok()?;
        let index: Self = serde_json::from_slice(&bytes).ok()?;
        (index.version == INDEX_VERSION).then_some(index)
    }

    pub(crate) fn write_for(path: &Path, offset: u64) -> io::Result<()> {
        let index = Self {
            version: INDEX_VERSION,
            offset,
        };
        let mut bytes = serde_json::to_vec(&index).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let index_path = index_path(path);
        let parent = index_path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(index_path)
            .map(|_| ())
            .map_err(|error| error.error)
    }
}

fn index_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".resume-index");
    PathBuf::from(name)
}
