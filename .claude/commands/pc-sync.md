---
description: Send the working tree to the dev PC, then build and deploy it (tools/mac/pc sync, build, deploy), between the owner's play sessions.
argument-hint: "[--bundle] [--allow-large] [--force]"
---
Ship the current working tree to the PC and, when asked, build and deploy it:

1. Run `tools/mac/pc status` and continue only on IDLE (exit 0). Otherwise work on the Mac. `--force` needs the owner's OK in this conversation.
2. Run `tools/mac/pc sync $ARGUMENTS`. It snapshots the index, your changes and the untracked files that are not ignored, and sends them to `C:\kairo\src` on the PC, branch `snapshot`.
   - It pushes with git (updateInstead) and falls back to a git bundle copied with scp; `--bundle` skips the push.
   - Your branches, index and stash stay as they are. The snapshot goes to the PC only, never to GitHub.
   - Ignored folders (`.dogfood/`, `.private/`, `target/`) never leave the Mac.
   - Untracked files over 50 MiB are refused: add them to `.gitignore`, or pass `--allow-large`.
3. Run `tools/mac/pc build`, or `tools/mac/pc build -p <package>` for one package. It runs `cargo build --release --locked` at below-normal priority and stops by itself if the owner becomes active (exit 3). It refuses while `Cargo.lock` is not in the snapshot, because this build runs every dependency's build scripts as an administrator: after a dependency change, commit the updated `Cargo.lock` first. First builds take minutes, so give the Bash call a long timeout. On failure it prints the end of the log; `tools/mac/pc logs build-<utc>` shows more.
4. Run `tools/mac/pc deploy`. It copies the build to `C:\kairo\bin\<build-id>`, points `C:\kairo\bin\current` at it, restarts the watch agent and keeps 3 builds. `tools/mac/pc rollback` returns to the previous build.
5. After changing the PC scripts in `tools/pc/`, run `tools/mac/pc doctor`. Before syncing, `tools/mac/pc doctor --local` runs the Mac's copy instead.

Never deploy while a job runs or the owner plays. Report the snapshot id, the build result and the build id in one or two sentences. Ask the owner a plain question when the problem is the PC itself rather than the code.
