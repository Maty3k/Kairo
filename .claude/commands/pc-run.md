---
description: Run a program in the owner's Windows session through \Kairo\Run (tools/mac/pc run). Never while the owner plays.
argument-hint: "<job.json | preset> [--no-wait | --wait SECONDS] [--force]"
---
Run one job on the dev PC with `tools/mac/pc run`:

1. Run `tools/mac/pc status` and continue only on IDLE (exit 0). On BUSY or UNKNOWN, stop and work on the Mac. Add `--force` only if the owner said in this conversation that you may use the PC now.
2. Choose the job: a preset from `tools/mac/jobs/` (`hello`), or a JSON file you write under `.dogfood/` (git-ignored):
   `{"id": "m1-probe-1", "cmd": ["C:\\kairo\\bin\\current\\probe.exe", "--seconds", "30"], "cwd": null, "max_seconds": 120, "abort_on_input": true, "window": "hidden"}`
   - `id` must match `^[A-Za-z0-9_-]{1,64}$`. It names `C:\kairo\logs\<id>.result.json`, `<id>.out.log` and `<id>.err.log`.
   - Only `id`, `cmd` and `max_seconds` are required. Keep `abort_on_input` true (the default) unless the owner should use the window. Keep `max_seconds` short.
   - Jobs run as the owner, who can read all of `C:\kairo` but write only `C:\kairo\logs` and `C:\kairo\state`. A null `cwd` means `C:\kairo`, so a program that writes files in its working folder needs `"cwd": "C:\\kairo\\logs"`.
   - Follow the house rules in CLAUDE.md: nothing inside games, no hooks, no injection, no synthetic input. Run anything in an anti-cheat game for the first time only with the owner present.
3. Run `tools/mac/pc run $ARGUMENTS`. It refuses while the PC is busy (exit 3 or 4). Otherwise it writes `C:\kairo\jobs\next.json`, starts `\Kairo\Run`, waits up to `max_seconds` + 30 s and prints the result and output. Give the Bash call a longer timeout than that wait.
   - Exit 0 ok, 1 failed, 5 still running (read it later with `tools/mac/pc logs <id>`).
   - Exit 6 means kairo-devrun stopped the job at its time limit or at the owner's input. That is the safety stop working, not a bug.
4. For long jobs, use `--no-wait` and read the result later with `tools/mac/pc logs <id>`.

The M0 hello check: the owner sits at the PC, so the status is BUSY. Ask them first, then:

- run `tools/mac/pc run hello --force --no-wait`. "Hello from Kairo" opens on the main monitor for up to 5 minutes;
- ask the owner to press a key in that window;
- run `tools/mac/pc cli keys --force` to read the key over the pipe, or `tools/mac/pc logs hello --force` to read hello.log;
- run `tools/mac/pc cli ping --force` to check that the pipe answers from SSH.

Report what ran and how it ended in plain words. Logs can contain PC details, so summarize them instead of pasting them into committed files.
