// SPDX-License-Identifier: GPL-3.0-or-later
//! Pruning old run folders (docs/research/16-verify-testbed/README.md §8.3 phase 0): the newest
//! 5 `harness-<stamp>` folders in the logs folder stay, the current run's among them, and the
//! older ones go. Only real folders named like a run count; a link (a junction reads as one) is
//! never followed or removed, and nothing else in the logs folder is touched. `kairo-devrun`
//! prunes its own job logs.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::cli::valid_run_id;

/// How many run folders stay.
pub const KEEP: usize = 5;

/// What pruning did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Pruned {
    /// Folders removed.
    pub removed: usize,
    /// Run folders kept.
    pub kept: usize,
    /// Folders that couldn't be removed: counted, never fatal.
    pub failed: usize,
}

/// Of the run folders `names`, the ones to remove: all but the newest `keep`, by name (the stamp
/// sorts by time).
pub fn select(names: &[String], keep: usize) -> Vec<String> {
    let mut runs: Vec<&String> = names.iter().filter(|n| valid_run_id(n)).collect();
    runs.sort_unstable_by(|a, b| b.cmp(a));
    runs.into_iter().skip(keep).cloned().collect()
}

/// Prunes `logs`, keeping `keep` run folders.
pub fn prune(logs: &Path, keep: usize) -> Pruned {
    let Ok(entries) = fs::read_dir(logs) else {
        return Pruned::default();
    };
    let names: Vec<String> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let kind = e.file_type().ok()?;
            (kind.is_dir() && !kind.is_symlink())
                .then(|| e.file_name().into_string().ok())
                .flatten()
        })
        .collect();
    let old = select(&names, keep);
    let runs = names.iter().filter(|n| valid_run_id(n)).count();
    let mut pruned = Pruned {
        kept: runs - old.len(),
        ..Pruned::default()
    };
    for name in old {
        let path = logs.join(&name);
        // Checked again just before: a folder swapped for a link since isn't followed.
        let real = fs::symlink_metadata(&path)
            .is_ok_and(|m| m.file_type().is_dir() && !m.file_type().is_symlink());
        if real && fs::remove_dir_all(&path).is_ok() {
            pruned.removed += 1;
        } else {
            pruned.failed += 1;
        }
    }
    pruned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_newest_five_runs_stay() {
        let mut list: Vec<String> = (0..8)
            .map(|n| format!("harness-2026111{}-120000", n))
            .collect();
        list.extend(names(&[
            "harness-displays",
            "harness-20261111-12000x",
            "build-20261111-120000",
            "foo",
        ]));
        assert_eq!(
            select(&list, 5),
            names(&[
                "harness-20261112-120000",
                "harness-20261111-120000",
                "harness-20261110-120000",
            ])
        );
        assert!(select(&list[..3], 5).is_empty());
    }

    #[test]
    fn prunes_real_run_folders_only() {
        let dir = std::env::temp_dir().join(format!("kairo-harness-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for n in 0..7 {
            let run = dir.join(format!("harness-2026111{n}-120000"));
            fs::create_dir(&run).unwrap();
            fs::write(run.join("harness.json"), "{}").unwrap();
        }
        fs::create_dir(dir.join("harness-displays")).unwrap();
        fs::write(dir.join("harness-20261101-000000"), "a file, not a run").unwrap();
        // A link named like an old run, to a folder that must survive.
        let precious = dir.join("precious");
        fs::create_dir(&precious).unwrap();
        fs::write(precious.join("keep.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&precious, dir.join("harness-20261100-000000")).unwrap();
        let p = prune(&dir, 5);
        assert_eq!(
            p,
            Pruned {
                removed: 2,
                kept: 5,
                failed: 0
            }
        );
        assert!(!dir.join("harness-20261110-120000").exists());
        assert!(dir.join("harness-20261112-120000").exists());
        assert!(dir.join("harness-displays").exists());
        assert!(dir.join("harness-20261101-000000").is_file());
        assert!(precious.join("keep.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
