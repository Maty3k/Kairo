// SPDX-License-Identifier: GPL-3.0-or-later
//! `ready.json` (docs/research/16-verify-testbed/README.md §4.10): written once the first frame
//! is presented, so a harness knows where the window is and when frame 0 was drawn for. It
//! names the window handle and the adapter of this run, which are details of the PC, so it stays
//! local (`C:\kairo\logs`, `.dogfood/`), like the log.

use std::fs;
use std::io;
use std::path::Path;

use serde_json::json;

/// What the ready file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    /// The testbed's process id.
    pub pid: u32,
    /// The window handle, as a number.
    pub hwnd: isize,
    /// The client area in desktop pixels: left, top, width, height.
    pub client: (i32, i32, i32, i32),
    /// The monitor's refresh rate, in mHz.
    pub refresh_mhz: u64,
    /// The adapter's LUID, as `0x` and 16 hex digits.
    pub adapter_luid: String,
    /// The adapter's DXGI index, valid in this process only.
    pub adapter_index: u32,
    /// The monitor's DXGI output index on it, valid in this process only.
    pub output_index: Option<u32>,
    /// `bgra8` or `fp16`.
    pub format: &'static str,
    /// `borderless`, `windowed` or `exclusive`.
    pub mode: &'static str,
    /// Frames per second.
    pub fps: u32,
    /// Frame 0's target time, QPC in 100 ns.
    pub start_hns: i64,
}

impl Ready {
    /// The file's text: one JSON object.
    pub fn json(&self) -> String {
        let (left, top, width, height) = self.client;
        json!({
            "kairo_testbed_ready": 1,
            "pid": self.pid,
            "hwnd": self.hwnd,
            "client": {"left": left, "top": top, "width": width, "height": height},
            "refresh_mhz": self.refresh_mhz,
            "adapter_luid": self.adapter_luid,
            "dxgi_adapter_index": self.adapter_index,
            "dxgi_output_index": self.output_index,
            "format": self.format,
            "mode": self.mode,
            "fps": self.fps,
            "start_hns": self.start_hns,
        })
        .to_string()
            + "\n"
    }

    /// Writes the file whole: into `<path>.partial`, then renamed, so a reader never sees half
    /// of it.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut partial = path.as_os_str().to_owned();
        partial.push(".partial");
        fs::write(&partial, self.json())?;
        fs::rename(&partial, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ready_file_is_one_json_object() {
        let ready = Ready {
            pid: 4_242,
            hwnd: 0x0003_04A2,
            client: (-1_920, 0, 1_920, 1_080),
            refresh_mhz: 179_960,
            adapter_luid: "0x0000000000012345".to_owned(),
            adapter_index: 1,
            output_index: Some(0),
            format: "bgra8",
            mode: "borderless",
            fps: 180,
            start_hns: 123_450_000_000,
        };
        let text = ready.json();
        assert!(text.ends_with("}\n"));
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["kairo_testbed_ready"], 1);
        assert_eq!(value["client"]["left"], -1_920);
        assert_eq!(value["start_hns"], 123_450_000_000_i64);
        assert_eq!(value["dxgi_output_index"], 0);
        let dir = std::env::temp_dir().join(format!("kairo-ready-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ready.json");
        ready.write(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(!dir.join("ready.json.partial").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
