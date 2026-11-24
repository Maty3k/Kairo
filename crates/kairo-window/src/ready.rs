// SPDX-License-Identifier: GPL-3.0-or-later
//! The create-new ready writer (docs/research/16-verify-testbed/README.md §4.10, research 17
//! §3.1). A program writes its ready file once its first frame is on its way, so the harness or
//! the bench knows where its window is; the file's JSON is the program's own (the testbed's
//! `ready.json`, loadgen's). It names details of the PC, so it stays local (`C:\kairo\logs`,
//! `.dogfood/`), like the logs.

use std::fs;
use std::io;
use std::path::Path;

/// Writes `text` to `path` whole, and only if nothing is there yet: the text goes into
/// `<path>.<pid>.partial`, which is linked in as `path` (a step that fails with `AlreadyExists`
/// if a file got there first, where a rename would replace it) and then removed. A reader never
/// sees half of it. `pid` is the writer's process id, so two writers never share a partial file.
pub fn write_new(path: &Path, pid: u32, text: &str) -> io::Result<()> {
    let mut partial = path.as_os_str().to_owned();
    partial.push(format!(".{pid}.partial"));
    fs::write(&partial, text)?;
    let linked = fs::hard_link(&partial, path);
    let _ = fs::remove_file(&partial);
    linked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_file_is_written_whole_and_only_once() {
        let dir = std::env::temp_dir().join(format!("kairo-window-ready-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ready.json");
        let text = "{\"ready\":1}\n";
        write_new(&path, 4_242, text).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(!dir.join("ready.json.4242.partial").exists());
        // A second writer given the same file leaves the first one's alone.
        let err = write_new(&path, 77, "{\"ready\":2}\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(!dir.join("ready.json.77.partial").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
