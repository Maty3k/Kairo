import json, sys, time, urllib.request
crates = """windows 0.62.2
windows-future 0.3.2
windows-registry 0.6.1
ffmpeg-next 9.0.0
ffmpeg-sys-next 9.0.1
wasapi 0.26.0
rubato 5.0.1
rtrb 0.3.5
crossbeam-channel 0.5.17
bytemuck 1.25.2
tokio 1.54.0
tokio-util 0.7.20
futures 0.3.34
bytes 1.12.1
serde 1.0.229
serde_json 1.0.154
ts-rs 12.0.1
schemars 1.2.3
tracing 0.1.44
tracing-subscriber 0.3.23
tracing-appender 0.2.5
tracing-log 0.2.0
file-rotate 0.8.0
sentry 0.49.3
crash-handler 0.8.1
minidumper 0.11.0
minidump-writer 0.13.0
rusqlite 0.40.2
rusqlite_migration 2.6.0
tray-icon 0.26.1
muda 0.21.2
steamlocate 2.1.1
roxmltree 0.21.1
embed-resource 3.0.12
embed-manifest 1.5.1
ulid 3.0.0
anyhow 1.0.104
thiserror 2.0.21
dirs 7.0.0
semver 1.0.28
tauri 2.12.4
tauri-build 2.7.3
tauri-plugin-single-instance 2.5.4
tauri-plugin-window-state 2.5.0
tauri-plugin-opener 2.7.2
tauri-plugin-dialog 2.8.3
tauri-plugin-updater 2.14.0
tauri-plugin-process 2.4.0
tauri-plugin-drag 2.1.1
tauri-plugin-wdio-webdriver 1.5.0
http-range 0.1.6
percent-encoding 2.3.2
arboard 3.7.0
cargo-xwin 0.23.1
dump_syms 2.3.9
minidump-stackwalk 0.27.0
proptest 1.9.0
tauri 2.12.5
ffmpeg-next 9.0.1""".strip().splitlines()
for line in crates:
    name, ver = line.split()
    url = f"https://crates.io/api/v1/crates/{name}/{ver}"
    req = urllib.request.Request(url, headers={"User-Agent": "kairo-cookbook-check (+https://github.com/Maty3k/Kairo)"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            d = json.load(r)["version"]
            print(f"{name}\t{d['num']}\t{d['license']}\t{d['created_at'][:10]}\t{d.get('rust_version')}\tyanked={d['yanked']}")
    except Exception as e:
        print(f"{name}\t{ver}\tERROR {e}")
    time.sleep(0.25)
