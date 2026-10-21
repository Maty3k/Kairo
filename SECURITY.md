# Security policy

## Report a vulnerability privately

Please don't report security problems in public issues: everything in this repository's issues is public.

Use GitHub's private vulnerability reporting instead:

1. Open the **Security and quality** tab of [Maty3k/Kairo](https://github.com/Maty3k/Kairo) and click **Report a vulnerability**, or go straight to <https://github.com/Maty3k/Kairo/security/advisories/new>.
2. Describe the problem: what is affected (file, component, commit or version), how to reproduce it, and what an attacker could do with it.
3. Click **Submit report**. Only you and the maintainers can see it.

GitHub's guide: [Privately reporting a security vulnerability](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/report-privately).

If the button is missing, or you can't use GitHub's form, open an issue titled "Security contact request" without any details, and we will arrange a private channel with you.

Please leave out personal data the report doesn't need, such as clips, screenshots or unredacted logs from your PC.

## What happens next

- We acknowledge your report within **72 hours**.
- We tell you whether we can reproduce the problem, keep you updated while we work on a fix, and agree a disclosure date with you.
- The fix ships in a new release, and the published advisory credits you unless you'd rather not be named.

## Supported versions

Kairo has no releases yet. Once it does, security fixes go into the next release only; older versions are not patched, so please stay on the latest one.

## Scope

In scope: everything in this repository, including:

- the Kairo app as it is built: the engine, the library app, `kairo-cli`, and later the installer and the updater;
- its local interfaces: the engine's named pipe, which is meant to accept only your Windows account and SYSTEM and to refuse remote clients, and the way the library app reaches your clips;
- the update and release path: release workflows, update manifests and their signatures, and the CI configuration;
- the development tools in `tools/`, such as the PC bootstrap script, which runs with administrator rights and sets up SSH;
- anything that breaks Kairo's privacy promises: data leaving the PC without your consent, network connections beyond the ones Kairo documents, or clips, audio or personal data in logs, crash reports or problem reports;
- anything that could get a player flagged by anti-cheat software: code running inside a game process, reads of a game's memory, or synthetic input. Kairo's house rules forbid all three ([CLAUDE.md](CLAUDE.md)).

Out of scope:

- problems in third-party software (Windows, graphics drivers, FFmpeg, WebView2, Rust crates, npm packages): please report those upstream, and tell us as well if Kairo ships an affected version or uses it in an exploitable way;
- attacks by someone who already has administrator rights or runs code as your own Windows account, unless Kairo gives them more than they already had;
- Kairo's future share-link service, which is a separate project and will get its own security contact when it launches.
