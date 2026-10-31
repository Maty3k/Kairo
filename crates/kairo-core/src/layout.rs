// SPDX-License-Identifier: GPL-3.0-or-later
//! The PC folder layout of the M0 contract (docs/PLAN.md §3.1).
//!
//! Everything Claude puts on the PC lives under `C:\kairo`:
//!
//! - `src\`: a non-bare clone (`receive.denyCurrentBranch=updateInstead`, branch `snapshot`);
//! - `bin\<build-id>\`: builds, with `bin\current` a junction to the active one (3 are kept);
//! - `jobs\`, `logs\`, `tools\` (FFmpeg, PresentMon);
//! - `state\`: `watch.json` and `bootstrap-report.json`;
//! - `build\target\`: the output of `pc build`, which runs as the owner (M1).
//!
//! Paths are Windows paths built as strings, so this module behaves the same in tests on any OS.

/// The root folder on the PC.
pub const DEFAULT_ROOT: &str = r"C:\kairo";

/// Every M0 path under one root: `C:\kairo` on the PC, a temporary folder in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self::new(DEFAULT_ROOT)
    }
}

impl Layout {
    /// A layout under `root`; trailing path separators are ignored.
    pub fn new(root: impl Into<String>) -> Self {
        let mut root = root.into();
        while root.len() > 3 && (root.ends_with('\\') || root.ends_with('/')) {
            root.pop();
        }
        Self { root }
    }

    /// The root folder, e.g. `C:\kairo`.
    pub fn root(&self) -> &str {
        &self.root
    }

    fn join(&self, relative: &str) -> String {
        format!(r"{}\{relative}", self.root)
    }

    /// `src`: the clone Claude pushes working-tree snapshots to.
    pub fn src_dir(&self) -> String {
        self.join("src")
    }

    /// `bin`: one folder per build.
    pub fn bin_dir(&self) -> String {
        self.join("bin")
    }

    /// `bin\current`: a directory junction to the active build.
    pub fn current_bin_dir(&self) -> String {
        self.join(r"bin\current")
    }

    /// `jobs`: job files for `\Kairo\Run`.
    pub fn jobs_dir(&self) -> String {
        self.join("jobs")
    }

    /// `logs`: job results and output, `hello.log`, `devrun.log`.
    pub fn logs_dir(&self) -> String {
        self.join("logs")
    }

    /// `tools`: FFmpeg, PresentMon and the PowerShell helpers.
    pub fn tools_dir(&self) -> String {
        self.join("tools")
    }

    /// `state`: files other tools poll, such as `watch.json`.
    pub fn state_dir(&self) -> String {
        self.join("state")
    }

    /// `jobs\next.json`: the job `\Kairo\Run` runs.
    pub fn next_job_file(&self) -> String {
        self.join(r"jobs\next.json")
    }

    /// `state\watch.json`: written every second by `kairo-devrun watch`.
    pub fn watch_file(&self) -> String {
        self.join(r"state\watch.json")
    }

    /// `state\bootstrap-report.json`: written by `tools\pc\bootstrap.ps1`.
    pub fn bootstrap_report_file(&self) -> String {
        self.join(r"state\bootstrap-report.json")
    }

    /// `logs\hello.log`: one JSON line per event of `kairo-hello`.
    pub fn hello_log_file(&self) -> String {
        self.join(r"logs\hello.log")
    }

    /// `logs\devrun.log`: one JSON line per event of `kairo-devrun`.
    pub fn devrun_log_file(&self) -> String {
        self.join(r"logs\devrun.log")
    }

    /// `logs\<id>.result.json`. The id must be valid ([`crate::job::is_valid_job_id`]).
    pub fn job_result_file(&self, id: &str) -> String {
        self.join(&format!(r"logs\{id}.result.json"))
    }

    /// `logs\<id>.out.log`: the job's standard output.
    pub fn job_stdout_file(&self, id: &str) -> String {
        self.join(&format!(r"logs\{id}.out.log"))
    }

    /// `logs\<id>.err.log`: the job's standard error, or why it could not run.
    pub fn job_stderr_file(&self, id: &str) -> String {
        self.join(&format!(r"logs\{id}.err.log"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_matches_the_m0_contract() {
        let layout = Layout::default();
        assert_eq!(layout.root(), r"C:\kairo");
        assert_eq!(layout.src_dir(), r"C:\kairo\src");
        assert_eq!(layout.bin_dir(), r"C:\kairo\bin");
        assert_eq!(layout.current_bin_dir(), r"C:\kairo\bin\current");
        assert_eq!(layout.jobs_dir(), r"C:\kairo\jobs");
        assert_eq!(layout.logs_dir(), r"C:\kairo\logs");
        assert_eq!(layout.tools_dir(), r"C:\kairo\tools");
        assert_eq!(layout.state_dir(), r"C:\kairo\state");
        assert_eq!(layout.next_job_file(), r"C:\kairo\jobs\next.json");
        assert_eq!(layout.watch_file(), r"C:\kairo\state\watch.json");
        assert_eq!(
            layout.bootstrap_report_file(),
            r"C:\kairo\state\bootstrap-report.json"
        );
        assert_eq!(layout.hello_log_file(), r"C:\kairo\logs\hello.log");
        assert_eq!(layout.devrun_log_file(), r"C:\kairo\logs\devrun.log");
        assert_eq!(
            layout.job_result_file("m0-1"),
            r"C:\kairo\logs\m0-1.result.json"
        );
        assert_eq!(
            layout.job_stdout_file("m0-1"),
            r"C:\kairo\logs\m0-1.out.log"
        );
        assert_eq!(
            layout.job_stderr_file("m0-1"),
            r"C:\kairo\logs\m0-1.err.log"
        );
    }

    #[test]
    fn trailing_separators_are_ignored_but_a_drive_root_is_kept() {
        assert_eq!(Layout::new(r"D:\tmp\k\").logs_dir(), r"D:\tmp\k\logs");
        assert_eq!(Layout::new("D:/tmp/k//").root(), "D:/tmp/k");
        assert_eq!(Layout::new(r"C:\").root(), r"C:\");
    }
}
