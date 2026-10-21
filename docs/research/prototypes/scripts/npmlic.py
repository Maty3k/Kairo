import json, urllib.request, urllib.parse
pk = """react 19.3.1
@tauri-apps/api 2.12.4
@tauri-apps/cli 2.12.3
@crabnebula/tauri-plugin-drag 2.1.0
@tanstack/react-query 5.104.5
@tanstack/react-virtual 3.14.21
@tanstack/react-router 1.170.45
zustand 5.0.15
radix-ui 1.8.1
tailwindcss 4.3.3
lucide-react 1.61.0
motion 14.5.0
@fontsource-variable/inter 5.3.0
wavesurfer.js 8.0.3
react-image-crop 11.1.2
i18next 26.5.4
react-i18next 17.1.0
sonner 2.0.8
react-hotkeys-hook 5.3.3
vite 8.3.5
typescript 6.0.3
vitest 5.0.5
webdriverio 9.32.0
@wdio/tauri-service 1.5.0
axe-core 4.14.0""".strip().splitlines()
for line in pk:
    name, ver = line.rsplit(" ", 1)
    url = "https://registry.npmjs.org/" + urllib.parse.quote(name, safe="@") + "/" + ver
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "kairo-cookbook-check"}), timeout=20) as r:
            d = json.load(r)
            print(f"{name}\t{d.get('version')}\t{d.get('license')}\tpeer={d.get('peerDependencies')}")
    except Exception as e:
        print(f"{name}\t{ver}\tERROR {e}")
for name in ["webdriverio"]:
    with urllib.request.urlopen(urllib.request.Request("https://registry.npmjs.org/"+name, headers={"User-Agent":"x"}), timeout=30) as r:
        d = json.load(r); print(name, "dist-tags", d.get("dist-tags"))
