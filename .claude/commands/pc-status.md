---
description: Check whether Kairo's Windows dev PC is free for remote work (tools/mac/pc status). Do this before any sync, build, deploy or run.
argument-hint: "[--json]"
allowed-tools: Bash(tools/mac/pc status) Bash(tools/mac/pc status *)
---
Check the dev PC's busy state from the repo root:

    tools/mac/pc status $ARGUMENTS

Read the `verdict:` line and the exit code:

- **0 IDLE**: no owner input for 10 minutes and nothing fullscreen. Remote work is allowed. Check again before each long step.
- **3 BUSY**: the owner may be playing. Don't sync, build, deploy or run anything on the PC; keep working on the Mac. Use `--force` on other verbs only if the owner said in this conversation that the PC is free now.
- **4 UNKNOWN**: the watch agent isn't reporting. In early M0 it isn't deployed yet; later it may have stopped, or the owner signed out. Treat this as busy, and ask the owner before using `--force`.
- **255**: ssh could not connect. The PC may be off or asleep, or the IP may have changed. On a first connection, pin the host key with `tools/mac/ssh-setup.sh <ip> <user> <SHA256:fingerprint>`, using the four lines the owner sent from the bootstrap's DONE screen (the *Host key* line is the fingerprint). Use `--trust-on-first-use` only when that line is missing, only on the home network right after the bootstrap, and have the owner compare the fingerprint it prints (on the PC: `ssh-keygen -lf $env:ProgramData\ssh\ssh_host_ed25519_key.pub`). A host key that changes later is never accepted automatically: ask the owner before removing the old one.

Capture and GUI tests need the owner signed in with the screen unlocked. The `session:` line shows this; LogonUI.exe running means the screen is locked.

Report the result in one plain sentence, for example "The PC is free: no input for 25 minutes." Keep PC details such as the IP, user name and program names out of committed files. Local notes go in `.dogfood/`.
