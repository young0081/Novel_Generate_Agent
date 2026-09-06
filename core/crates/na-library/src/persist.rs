use std::io::Write;
use std::path::Path;

use na_common::{CoreError, Result};

pub(crate) fn atomic_write(path: &Path, content: &[u8], description: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| {
        CoreError::from(error).with_context(format!("creating directory for {description}"))
    })?;

    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        CoreError::from(error).with_context(format!("creating temporary {description}"))
    })?;
    temp.write_all(content).map_err(|error| {
        CoreError::from(error).with_context(format!("writing temporary {description}"))
    })?;
    temp.as_file().sync_all().map_err(|error| {
        CoreError::from(error).with_context(format!("flushing temporary {description}"))
    })?;
    temp.persist(path).map_err(|error| {
        CoreError::from(error.error).with_context(format!("replacing {description}"))
    })?;
    Ok(())
}
