# Setting up your gaming PC for Kairo

This is the one-time setup that lets Claude build and test Kairo on your Windows PC (milestone M0 in [docs/PLAN.md](docs/PLAN.md#m0--setup-and-bridge-s); the checklist lives in [docs/milestones/M0.md](docs/milestones/M0.md)). It is for Kairo's development on your own PC only, not for friends or testers.

**Time:** about 1 to 1.5 hours, once. Longer if Windows needs a big update first.
**You need:** your normal Windows account, and it must be an **administrator** (Settings → Accounts → Your info says "Administrator"), and about **40 GB** free on `C:` (the setup stops below 15 GB). Claude sends you one command in chat; you paste it and press **Y**.

---

## Step by step

### 1. Update Windows

- **Settings → Windows Update:** install everything and restart if asked.
- **Settings → System → About** shows your Windows **Version**:
  - 24H2 or older: take the **25H2** update when Windows Update offers it (24H2 Home and Pro stopped getting updates on 2026-10-13).
  - 25H2 or 26H2: stay where you are. Later big updates are taken on purpose, not automatically.
- **Windows Update → Advanced options → Active hours:** set your evenings and nights, so Windows doesn't restart while you play. Claude will ask you to pause updates during soak-test weeks.

### 2. Set your network to Private

**Settings → Network & internet →** click your connection (Ethernet or Wi-Fi) **→ Network profile type: Private.**

The Mac can only reach the PC on a Private network. The setup never changes this setting for you. Do this before step 4: SSH only opens on the adapters that were on a Private network when the setup last ran, so if you change this later, or switch between wired and Wi-Fi, run the setup command again.

### 3. Power settings

- **Settings → System → Power:** set sleep to **Never** while Claude has PC work (until milestone M9).
- On days you agree that Claude may test while you're away, turn off Dynamic Lock and any screen-saver lock, and leave the PC on, signed in and unlocked.

### 4. Run the setup command

1. Claude sends you the exact command in chat. It contains nothing secret (a version number of the script and your Mac's *public* key), so messaging it to yourself is fine. Only a command from Claude in your own chat counts: never run one that anybody else sends you (see [The setup command](#the-setup-command)).
2. On the PC, **right-click Start → Terminal (Admin)** and click **Yes**. The tab should say *Administrator: Windows PowerShell*.
3. Paste the command (Ctrl+V) and press Enter.
4. The script shows what it found and lists everything it will install or change. Read it, then type **Y** and press Enter. Anything else stops it without changing anything.
5. Wait 20 to 40 minutes. The longest part is Visual Studio Build Tools; a *Visual Studio Installer* window shows its progress. If Windows asks *"Do you want to allow this app to make changes?"*, choose **Yes**.
6. It ends with **DONE** and four lines: **PC name**, **IP**, **User** and **Host key**. **DONE** is green when every step worked; if a step had a problem, it is yellow and lists those steps.

Steps with a red **PROBLEM** don't stop the others. Yellow **ATTENTION** lines are warnings: read them, because some ask you to do something (see [If something goes wrong](#if-something-goes-wrong)). The DONE screen lists them again, even when DONE is green. Running the same command again is safe: it skips the installs that are already done and checks the rest again. Two of its steps go through every file in `C:\kairo\src` again (*Prepare the code folder*, and the last step, which also covers `bin`, `jobs` and `tools`), so a re-run takes longer once Claude has made builds there.

### 5. Send Claude the four lines, then sign out and back in

Send Claude the PC name, IP, user name and host key from the DONE screen. The host key lets the Mac check, the first time it connects, that it really talks to your PC and not to another device on the network. Then **sign out of Windows and sign back in** (Start → your name → Sign out), or **restart** if the DONE screen says so. This switches on your new group membership and the task that starts at logon.

If the DONE screen says SSH was switched off again, a part of the SSH setup failed, and the setup turned OpenSSH Server off rather than leave it half set up. Tell Claude.

### 6. Smart App Control

**Start → type "Smart App Control" → open it** and tell Claude whether it says **On**, **Evaluation** or **Off**. (The setup also reads it, but never changes it.)

- **Off:** nothing to do.
- **On or Evaluation:** On blocks Kairo's test builds, because it checks every program, build tools included; Evaluation can switch itself to On later. Claude explains the two routes (question Q3 in the plan):
  - **(a) Turn it off.** Claude's recommendation. It is reportedly reversible since update KB5083769, though one report says KB5127648 greys the switch out (uncertain).
  - **(b) Keep it on and pay for signing** (Certum Open Source, €25, non-commercial only). Builds then happen off the PC and you approve each signature with a one-time code on your phone: slower fixes, and no longer free.

  M0 only closes when one of the two routes works.

### 7. AMD Instant Replay

Open **AMD Software: Adrenalin Edition → Record & Stream** and tell Claude whether **Instant Replay** is on, its length and its hotkeys. **Leave it on**: it stays your safety net until Kairo replaces it (milestone M5).

### 8. Tell Claude about your games

The games and launchers you play, which ones use HDR, any you run as administrator, your keyboard layouts, your usual play times, and the days the PC may run tests while you're away.

### 9. Hello test

When Claude asks, a window called **Hello from Kairo** opens on your main monitor. Click it and press any key. Claude tells you which key arrived; say whether it's right.

### 10. On the Mac (not the PC)

Free up disk space until at least **20 GB** is free (System Settings → General → Storage), then tell Claude. Claude then shows you Microsoft's licence for the Windows SDK files that the Mac's build tool downloads; accept it only if you agree.

### 11. Optional: faster builds

A **Dev Drive** for `C:\kairo`, or a Microsoft Defender exclusion for `C:\kairo\src\target` only, makes builds faster. Claude explains both; skipping them is fine.

---

## The setup command

Claude sends it to you filled in. Its shape is:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/Maty3k/Kairo/<COMMIT-SHA>/tools/pc/bootstrap.ps1))) -MacKey '<public key>'
```

- **Run only the command Claude sends you in your own chat.** A link of this shape can also point at code from someone else's copy of Kairo, so never run one that anybody else sends you.
- **`<COMMIT-SHA>`** names one exact version of [tools/pc/bootstrap.ps1](tools/pc/bootstrap.ps1). Claude only sends a commit that is part of Kairo's `main` branch and passed its automatic checks (CI).
- **`<public key>`** is the Mac's public SSH key, one line starting with `ssh-ed25519`. It lets the Mac log in; it is not a password, and nothing secret ever goes into the command.
- Neither value is stored in this repository.
- It must run in **Windows PowerShell** as administrator (Terminal (Admin)), not in PowerShell 7.

## What the setup changes

| What | Change | Why |
|---|---|---|
| Programs (installed with winget, Microsoft's package manager) | Git; Visual Studio Build Tools 2026 with the C++ tools and the Windows 11 SDK; LLVM; Rust (rustup, then Rust 1.99.1); Node.js 24 LTS and pnpm (through corepack); PowerShell 7.6 | Building Kairo on the PC |
| Downloads, each checked against a fixed SHA-256 fingerprint | FFmpeg 9.0.2 (LGPL build by BtbN) in `C:\kairo\tools\ffmpeg`; PresentMon 2.6.0 (Intel's frame-timing tool) in `C:\kairo\tools\presentmon` | Recording tests; measuring frame rates |
| OpenSSH Server (a Windows feature) | Turned on; **key logins only, passwords and keyboard-interactive logins off**, set before it first starts; it starts with Windows only once that is proven, and is switched off again if that step fails. The Mac's key goes into `C:\ProgramData\ssh\administrators_authorized_keys`, which only Administrators and SYSTEM can open. SSH sessions start in `cmd.exe` with its AutoRun off, and Claude's tools start PowerShell 7 without your profile | Claude works on the PC from the Mac |
| Windows Firewall | New rule *Kairo SSH*: TCP 22, only for OpenSSH's `sshd.exe`, only on the wired or Wi-Fi adapters that were on a Private network with a home-network address when the setup ran, only while they are on a Private network, only from your local network. If no adapter qualified, the rule is created switched off (fix step 2, then run the command again). Windows' broader default OpenSSH rule (named `OpenSSH-Server-In-TCP`, open on every network type) and any other rule opening port 22 are switched off; the setup names each one before you press **Y** | SSH only from your home network, never from the internet or a gaming VPN |
| Your account | Added to the *Performance Log Users* group | PresentMon can measure without admin rights |
| Folders | `C:\kairo` with `src`, `bin`, `jobs`, `logs`, `tools`, `state`. Administrators and SYSTEM have full control; you can read everything, and your own programs can write only `logs` and `state`. The setup's last step makes Administrators the owner of `C:\kairo` and of everything in `src`, `bin`, `jobs` and `tools`, and gives every file there the same permissions as its folder | Everything Kairo's development needs stays in one place, and nothing running as you can change what Claude's administrator sessions run (a file's owner can always change its permissions, which is why Administrators own these) |
| `C:\.cargo` | An empty folder that only administrators can change | Keeps any other program from planting a setting there that Claude's builds would read |
| System variables | `LIBCLANG_PATH` (to LLVM) and `FFMPEG_DIR` (to `C:\kairo\tools\ffmpeg`); `POWERSHELL_TELEMETRY_OPTOUT=1` | Building the parts of Kairo that use FFmpeg; PowerShell 7 sends no telemetry |
| Visual Studio usage data | Switched off before the Build Tools install: `OptIn` = 0 under `HKLM\Software\Policies\Microsoft\VisualStudio\SQM` (you agreed on 2026-10-23) | The Build Tools and their installer don't follow Windows' privacy settings. This stops their optional usage data; Microsoft still receives the data it calls required |
| Scheduled tasks, folder *Kairo* | `Run`, `Ensure`, `Engine`. They run as you (not as administrator), only while you're signed in, and do nothing until Claude installs Kairo's test programs. `Ensure` checks once a minute and exits at once | Starting test programs in your session, which SSH can't do |
| Git | `C:\kairo\tools\gitconfig`: Git settings for Claude's SSH sessions that mark `C:/kairo/src` as a safe folder. Your own Git settings are not changed, and those sessions never read them | Code arrives there from the Mac |
| Report | `C:\kairo\state\bootstrap-report.json`: Windows version, CPU, RAM, graphics card and driver, monitors, disk space, PC name, IP and network adapters, user name and account ID (SID), installed tool versions, the fingerprints of the Mac's key and of the PC's SSH host key, the SSH and firewall settings it made, Smart App Control and Instant Replay state (with the AMD recording and hotkey settings it found), and the result of each step with any warnings | Claude's starting facts; it stays on your PC and in a private, never-published folder on the Mac |

A log of the whole run is saved as `C:\kairo\logs\bootstrap-<date-time>.log`. If `C:\kairo` could not be set up, the log stays in your TEMP folder as `kairo-bootstrap-<date-time>.log`; the DONE screen shows where it is.

**What it never does:** it doesn't touch your games, anti-cheat software or AMD Software (it only reads AMD Software's version and its Instant Replay, recording and hotkey settings), doesn't change Smart App Control, Windows Update or your network type, doesn't create passwords or remote desktop access, and opens nothing to the internet. It only downloads the programs listed above and uploads nothing itself. The Microsoft tools it uses or installs can send their own diagnostic data. For winget (part of Windows' App Installer), **Settings → Privacy & security → Diagnostics & feedback** controls it. The setup switches off PowerShell 7's telemetry and Visual Studio's optional usage data (see the table).

## How to undo

In **Terminal (Admin)** run the undo script. Claude gives you the same kind of pinned command, and the same rule applies: run it only when Claude sends it in your own chat.

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/Maty3k/Kairo/<COMMIT-SHA>/tools/pc/uninstall-dev.ps1)))
```

It removes the *Kairo* scheduled tasks, the Kairo firewall rule and the Mac's key, so the Mac can no longer log in. It lists what it will do and waits for **Y**. Add **`-RemoveData`** at the end to also delete `C:\kairo` (with the Git settings for Claude's SSH sessions in it), the `FFMPEG_DIR` variable and the empty `C:\.cargo`. The script is in [tools/pc/uninstall-dev.ps1](tools/pc/uninstall-dev.ps1).

It deliberately keeps the installed programs and a few settings. To remove those too:

- **Programs:** Settings → Apps → Installed apps (Git, Visual Studio Build Tools 2026, LLVM, Node.js, PowerShell 7). Rust: run `rustup self uninstall`.
- **OpenSSH Server:** Settings → System → Optional features → OpenSSH Server → Remove.
- **Key-only SSH settings:** in Terminal (Admin), run `Copy-Item C:\ProgramData\ssh\sshd_config.before-kairo C:\ProgramData\ssh\sshd_config`, then `Restart-Service sshd`. Not needed if you remove OpenSSH Server.
- **SSH shell:** the values `DefaultShell`, `DefaultShellCommandOption` and `DefaultShellArguments` under `HKLM\SOFTWARE\OpenSSH` (Registry Editor).
- **Performance Log Users:** in Terminal (Admin), `Remove-LocalGroupMember -SID S-1-5-32-559 -Member $env:USERNAME`.
- **`LIBCLANG_PATH` and `POWERSHELL_TELEMETRY_OPTOUT`:** Settings → System → About → Advanced system settings → Environment Variables.
- **Visual Studio usage data:** to switch it back on, in Terminal (Admin) run `Remove-ItemProperty -Path HKLM:\SOFTWARE\Policies\Microsoft\VisualStudio\SQM -Name OptIn`.

Windows' own SSH firewall rule, and any other rule the setup switched off because it opened port 22 (it named them before you pressed **Y**), stay switched off on purpose: switching Windows' rule back on would open SSH on every network. To switch another program's rule back on: Start → *Windows Defender Firewall with Advanced Security* → Inbound Rules → right-click it → Enable Rule.

## If something goes wrong

- **"This window is not running as administrator":** close it, right-click Start → **Terminal (Admin)**, click Yes, and paste the command again.
- **"winget (Microsoft's package manager) is missing":** open the Microsoft Store, search for **App Installer**, update it, and run the command again.
- **"Please run this in Windows PowerShell 5.1":** Terminal opened PowerShell 7. Open a new tab with **Windows PowerShell** (the arrow next to the + tab) and paste again.
- **"someone else is signed in at the screen":** the window was opened with another administrator's account. Sign in to Windows with your own account (it must be an administrator) and run the command there.
- **"Less than 15 GB is free on C:":** free some space (Settings → System → Storage) and run the command again; nothing was changed.
- **Any red PROBLEM line:** finish the run, then send Claude a photo or screenshot of the window. Most problems can be fixed from the Mac once SSH works, and running the command again is always safe.
- **Any yellow ATTENTION line:** the setup carried on, but read it and do what it says (for example: set your network to Private, then run the command again). The DONE screen lists these lines again under *Warnings from earlier in the run*, so the photo or screenshot of it that you send Claude includes them.
- **"C:\kairo already exists, but this setup did not create it":** another program or account made that folder. If nothing in it matters to you, delete it; otherwise move it somewhere else. Then run the command again.
- **"SSH was switched OFF again":** the SSH part had a problem, so OpenSSH Server was turned off instead of being left half set up. Send Claude a photo of the window.
