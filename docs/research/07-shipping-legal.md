# 07 - Shipping Kairo on Windows in 2026: signing, Store, installers, updates, crash reporting, privacy and legal (as of 2026-10-21)

Researcher brief for the Kairo planning team. Kairo is a Windows-only, local-first instant-replay and screenshot app. Report 05 recommends a Tauri 2 (TypeScript) UI plus a separate always-on Rust engine, with FFmpeg as LGPL DLLs for AMF, QSV and NVENC. This report assumes that stack where it matters, and flags where Electron, .NET or libobs would change the answer.

**Confidence tags:** [H] = primary source checked today (vendor docs, pricing pages, official policies, GitHub/crates.io). [M] = secondary source (law-firm or press summary, vendor blog), an older primary doc, or my reading of a primary text. [L] = single weak source or inference. **None of this is legal advice.** The legal sections identify questions for a lawyer; they don't answer them.

**Method notes**
- Prices were read today from vendor shops and from the Azure Retail Prices API. The Azure pricing page itself renders "$-" placeholders.
- Microsoft Learn pages show an `ms.date` and an `updated_at`. Both are quoted where they matter.
- The EDPB guidelines PDF was parsed locally to quote paragraph 33 exactly.
- EUR-Lex blocked automated fetches. EU legal texts (CRA recitals, DSA, GDPR) were checked through mirrors and law-firm summaries and are tagged [M].
- I assume the developer may be an **individual resident in an EU member state** with no registered company. Where the answer differs for a company or a US/Canada resident, I say so.

---

## 0. TL;DR - what matters for Kairo

1. **An EU-based individual cannot use Microsoft's cheap signing service.**
   - Artifact Signing (formerly Trusted Signing) issues Public Trust certificates to *organizations* in the US, Canada, EU, UK, AU, NZ, JP, KR, SG, CH, NO and IL. *Individuals* must be in the **US or Canada** (quickstart updated 2026-10-08) [H].
   - Prices: Basic **$9.99/month** for 5,000 signatures, Premium **$99.99/month** for 100,000, overage **$0.005/signature** [H, Azure Retail Prices API].
   - It needs a **paid** Azure subscription. Free, trial and sponsored subscriptions are not supported [H].
2. **EV certificates no longer bypass SmartScreen.** Microsoft's own docs say so: "EV certificates no longer bypass SmartScreen… Paying a premium for EV solely to avoid SmartScreen warnings is no longer justified" (updated 2026-08-17) [H].
   - Every non-Store binary builds reputation from its file hash plus its publisher certificate.
   - Microsoft: it "can take several weeks and hundreds of clean installs from a wide audience" [H].
   - A real indie publisher reports SmartScreen prompts on **every** new release despite 10 years on the same certificate (June 2026) [M].
3. **The cheapest credible OV certificate for an EU individual is Certum** [H, shop prices today].
   - "Standard Code Signing in the Cloud": **from €209**. Individuals are allowed.
   - Certum's **Open Source** certificate (from €49 cloud, €25 as a code, €69 as a card set) is revoked "if it's used to sign commercially distributed software" [H]. That makes it unsuitable if Kairo will ever be monetized.
   - Every publicly trusted code-signing certificate now lives on an HSM or token, and since **2026-03-01** maximum validity is **460 days** (CA/B Forum ballot CSC-31) [H].
4. **The Microsoft Store is the zero-cost way to avoid SmartScreen entirely.**
   - Registration is now **free for individual *and* company accounts** via storedeveloper.microsoft.com (Learn page updated 2026-07-17) [H].
   - **MSIX** submissions are re-signed by Microsoft, hosted on its CDN and auto-updated [H].
   - **EXE/MSI** submissions must be signed by you with a cert chaining to a Trusted Root Program CA, install silently, and be offline, standalone and immutable per versioned URL [H].
   - Tauri only emits EXE/MSI [H]. Microsoft's **winapp CLI** (public preview, supports Tauri/Rust) can `pack` MSIX [H].
   - You **cannot convert** an Individual account to Company later [H].
5. **Updaters.**
   - The Tauri updater requires its own signing key ("cannot be disabled") and downloads full installers, with no deltas [H].
   - **Velopack** (MIT, 1.2.161, 2026-09-29) installs per-user to `%LocalAppData%`, ships zstd per-file **delta** updates and has a built-in Artifact Signing flag [H].
   - electron-updater uses NSIS blockmap differential updates by default [H].
   - MSIX gets 64 KB-block differential updates. Its web `ms-appinstaller:` protocol has been **disabled by default since Dec 2023** [H].
6. **Crash reports and telemetry need opt-in in the EU.**
   - EDPB Guidelines 2/2023 (v2.0, 2024-10-07), para 33: Art. 5(3) ePrivacy applies "when the accessing entity distributes software on the terminal equipment of the user that is stored and will then proactively call an … API endpoint over the network" [H text; application to crash reports is my reading, M].
   - **Sentry** pricing: Developer plan free (5k errors/month, 1 user), Team **$26/month billed annually** ($29 monthly) [H]. It has an EU (Frankfurt) region, and minidumps are **discarded after processing by default** [H/M].
7. **Legal hot spots for a capture tool.**
   - **(a) Marketing.** Microsoft's Game Content Usage Rules bar game content "in an app that you sell in an app store" and require permission to promote a commercial venture [H]. Riot "prohibit[s] the use of our IP in games and apps" [H]. **Don't put other publishers' footage in Kairo's store listing or ads without permission.**
   - **(b) Paywalls.** EA, Valve, Riot and Microsoft forbid paywalling clips of their games [H]. A future "premium-only viewing" feature would push users into violations.
   - **(c) Voice chat.** Recording other people's voice can be a crime in some EU states (e.g., Germany §201 StGB). Publishing clips takes them out of the GDPR household exemption (CJEU *Buivids*) [M]. **Default to recording only the user's own mic, on its own track.**
8. **EU product law now applies to software.**
   - **Cyber Resilience Act:** vulnerability and incident **reporting obligations have applied since 2026-09-11**, and full obligations apply from **2027-12-11** [H, ENISA].
   - CRA "commercial activity" includes monetizing, or requiring personal data beyond security/compatibility needs. Non-monetized FOSS is out of scope [M, recital text via mirror].
   - The new **Product Liability Directive** makes software a "product" for items placed on the market after **2026-12-09** [M].
9. **License choice drives costs.**
   - **libobs is GPL-2.0-or-later** [H], so using it means GPL for anything linked with it.
   - FFmpeg's LGPL route requires dynamic DLLs, shipping the source, no `--enable-gpl/nonfree`, and an EULA that doesn't forbid reverse engineering [H].
   - Choosing a real OSI license unlocks **SignPath Foundation** (free signing, with "SignPath Foundation" shown as publisher) and Certum's €49 OSS cert, but only while non-commercial.

**Cheapest credible path (EU individual):**
- **Private beta:** sign with Certum Standard Code Signing in the Cloud (~€209/yr), use a Tauri NSIS per-user install with the Tauri updater (or Velopack) on GitHub Releases, Sentry Developer (free, EU region), and opt-in crash reports.
- **Public free release:** add a **Store MSIX** listing via winapp CLI (free signing, no SmartScreen, Store auto-updates).
- **If and when you monetize:** register a company, open a *new* company Store account (free), and evaluate Artifact Signing ($9.99/month as an EU organization).
- **Total cash outlay to public release: about €209/year**, plus a domain for the privacy policy. See section 8.

---

## 1. Code signing

### 1.1 Why signing matters for Kairo specifically

| Mechanism | What it does to an unsigned / new Kairo build | Source |
|---|---|---|
| **SmartScreen AppRep** (files with Mark-of-the-Web, i.e. browser downloads) | Unsigned: "Windows protected your PC", and the user must click "More info → Run anyway". Signed but new: the same "unrecognized app" warning, but showing the verified publisher name. | Learn, SmartScreen reputation, ms.date 2026-05-04, updated 2026-08-17 [H] |
| **Smart App Control** (Windows 11) | "Will block execution of unsigned files unless the file has a positive reputation." It applies to **all executables, not just downloads**. Apps signed with a trusted CA certificate are allowed. RSA and ECC are both supported (doc ms.date 2026-09-25). | Learn, SmartScreen page; Learn, Sign your app for SAC [H] |
| SAC adoption is rising | Since the **April 2026** cumulative update **KB5083769** (builds 26100.8246 / 26200.8246), SAC can be turned back on without reinstalling Windows. A French report says the October 2026 update KB5127648 greys the toggle out (Microsoft-acknowledged). | topedia.com (Apr 2026) [M]; it-connect.fr (2026-10-18) [L, not fetched] |
| **Store EXE/MSI route** | The binary "and all of its PE files" must be signed with a cert chaining to a Microsoft Trusted Root Program CA. | Learn, MSI/EXE package requirements, updated 2026-08-24 [H] |
| **AV heuristics** | A tray app with global hotkeys, screen capture and audio capture looks like spyware to heuristics. Signing plus false-positive submissions are the standard mitigation. | Defender submission guide [H]; inference [L] |

**Nuance on updates [M]:**
- SmartScreen AppRep is consulted for files carrying the Mark-of-the-Web, which come from browser downloads and Explorer copies from shares.
- An in-app updater that downloads with its own HTTP client normally doesn't add MOTW. So the *first install* is where SmartScreen bites.
- Smart App Control, however, checks every executable. That is why every PE file must be signed, including FFmpeg DLLs you redistribute and the uninstaller.
- Source: Eric Lawrence (former SmartScreen PM), "Best Practices for SmartScreen AppRep", 2024-11-15.

### 1.2 SmartScreen in 2026: EV no longer bypasses

- Microsoft (SmartScreen page): "EV certificates no longer bypass SmartScreen. Years ago, signing files with an Extended Validation (EV) code signing certificate would result in positive SmartScreen reputation by default, but this behavior no longer exists." [H]
- Microsoft's "distribution feature status" page (last reviewed Aug 2026) dates this to **2024**: "Microsoft updated the Trusted Root Program requirements in 2024, removing EV-specific OIDs… no certificate type (OV or EV) grants an immediate bypass." [H]
- Reputation has two signals: **file hash** and **publisher certificate**. "Signing files using a trusted certificate can allow certificate reputation to build, potentially avoiding warnings on new files signed by the same trusted certificate." "There is no exact threshold, but it can take several weeks and hundreds of clean installs from a wide audience." [H]
- **Conflict:** Velopack's signing docs still say "prefer an EV (or Azure Artifact Signing) certificate for instant reputation" [H, docs.velopack.io]. That contradicts Microsoft's current docs. Trust Microsoft.
- **Field report (June 2026):** the Markdown Monster author ships weekly with "the same cert for 10 years". He says "I run into this issue every single time I upload a new release". Prompts fade after a few hours for his popular app. A newer, low-download product "may never clear the SmartScreen Reputation hurdle between updates" [M].
- **Practical rules** (Microsoft and Lawrence) [H/M]:
  - sign every release with one stable identity;
  - keep the certificate `Subject` identical on renewal;
  - never inject per-download data into signed files;
  - never modify files after signing;
  - don't sign anything that behaves like a PUA, because the certificate can earn **negative** reputation;
  - tell beta users to expect a prompt.
- **No manual fast-track for consumers:** "There is no need (or mechanism) to manually submit a file for SmartScreen reputation review for consumer endpoints" [H]. Defender *false positives* are a different process: submit at microsoft.com/wdsi/filesubmission "as a software developer" [H].

### 1.3 Microsoft Artifact Signing (formerly Trusted Signing)

| Item | Fact | Tag |
|---|---|---|
| Price | Basic **$9.99/month** (5,000 signatures/month, 1 certificate profile of each type). Premium **$99.99/month** (100,000 signatures, 10 profiles). Overage **$0.005/signature**. Retail Prices API rows effective 2024-06-01 under the old product name "Trusted Signing". Microsoft's code-signing comparison page also says "~$9.99/month". | [H] |
| Billing | Starts when you create the account and is **not pro-rated**. | [H] FAQ |
| Subscription | "Doesn't support free, trial, or sponsored Azure subscriptions." | [H] FAQ, updated 2026-10-20 |
| Eligibility | Public Trust: **organizations** in US, Canada, EU, UK, Australia, NZ, Japan, South Korea, Singapore, Switzerland, Norway, Israel. **Individual developers must be located in the US or Canada.** | [H] quickstart, updated 2026-10-08 |
| Individual validation | Identity comes from an Azure billing account of type *Individual*. ID check runs through AU10TIX and a Microsoft Authenticator Verified ID. | [H] |
| Org validation | 1–20 business days. Documents must be <12 months old. Three upload attempts. | [H] |
| Org history | A **3-year business history** requirement was reported by a volunteer moderator in Aug 2025. Current official docs don't list it. A brand-new company may still be rejected. | [L] |
| Name on cert | CN/O must be the **validated legal name**. No trading names or custom CN. | [H] FAQ |
| Certificates | **72-hour certificates renewed daily.** Durable identity is a per-profile EKU `1.3.6.1.4.1.311.97.*`. Keys are in FIPS 140-3 Level 3 HSMs. Timestamp at `http://timestamp.acs.microsoft.com` (mandatory in practice). | [H] concept page, ms.date 2026-09-30 |
| EV | "Artifact Signing doesn't issue EV certificates… no plan to." | [H] |
| **Trust-root change** | "Artifact Signing code-signing CAs are scheduled for removal from the CCADB. Although Microsoft no longer includes these CAs in the Trusted Root Program, they remain on the Windows Platform Trust List." | [H] concept page, 2026-09-30 |
| Tooling | SignTool + dlib, the GitHub Action, Tauri `signCommand` with `artifact-signing-cli`, Velopack `--azureTrustedSignFile` (works from macOS/Linux since Velopack 1.3, which is still a prerelease as of 2026-10-17), `winapp az-sign`. | [H] |

**Two implications for Kairo:**
- **(a)** An EU *individual* is ineligible. An EU *company* is eligible, but the certificate shows the company's legal name.
- **(b) Open question [L]:** Store policy 10.2.9 demands a cert chaining to a *Trusted Root Program* CA for EXE/MSI submissions. Artifact Signing's CAs are now only on the Windows Platform Trust List. Microsoft's comparison table marks Artifact Signing "Store eligible: ❌ No", which is ambiguous.
  - If you want the Store EXE/MSI route, sign with a traditional CA (Certum, SSL.com, Sectigo) or confirm with Partner Center support first.
  - The Store **MSIX** route avoids the question, because Microsoft re-signs.

### 1.4 OV / EV certificates from CAs (prices read 2026-10-21)

**Key storage:** since 2023-06-01, private keys for publicly trusted code-signing certs must sit on a hardware token or HSM. Software-only keys are gone. Most CAs offer a USB token or a cloud HSM [H, Microsoft code-signing options; Sectigo: "software-only key storage isn't permitted"].

**Validity:** the max is **460 days since 2026-03-01** (CA/B Forum CSC-31, adopted 2025-11-17). CAs issue a bit below that: Sectigo 459 days (since 2026-02-23) and SSL.com 458 days [H/M]. "Multi-year" plans re-issue roughly yearly, and Sectigo ships a new token each year.

| Vendor / product | Price | Key storage | Individuals? | Notes |
|---|---|---|---|---|
| **Certum Open Source Code Signing** | **€25** (the "Open Source Code Signing - code" listing; no card or reader), **€49** (cloud/SimplySign, "out of stock" today), **€69** (card + reader set) | SimplySign cloud or cryptoCertum card | **Individuals only** | CN/O = "Open Source Developer" + your name. Needs a public OSS project URL. **Revoked if used for commercially distributed software.** SimplySign usage limit is 5,000 signatures/month. "Trusted by Microsoft, including for building SmartScreen reputation." [H] |
| **Certum Standard (OV) Code Signing** | **€209** cloud; **€169** card set; **€139** electronic code | SimplySign cloud HSM (mobile-app OTP) or card | **Yes** (ID + utility bill) | Cheapest OV option for an EU individual that I found. Shop shows "from" prices with gross = net, so VAT treatment at checkout is unverified [H] |
| Certum EV | €379 cloud / €359 set / €329 code | cloud or card | n/v | No SmartScreen benefit any more [H] |
| **SSL.com OV / IV** | **$129/yr** (1-yr term) down to $96.75/yr (5-yr) | **+$379 YubiKey** one-time, or eSigner cloud: **$20/month for 20 signings** ($180/yr for 240), $1 per extra signing | **Yes (IV, "no business docs required")** | Realistic cloud total ≈ **$309/yr**. 20 signings/month is tight when every PE file counts [H] |
| SSL.com EV | $349/yr (1-yr) down to $149/yr (5-yr) | YubiKey $379 or eSigner EV $100/month (10 signings) | Sole-proprietor EV exists | Not worth it for SmartScreen [H] |
| Sectigo (direct) | "Starts at **$536.25 per year** when customers choose the five-year option" | Shipped FIPS token, or your own HSM | Unclear | Resellers list Sectigo OV from about $219/yr [M/L, reseller pages] |
| Microsoft's own ranges | OV "$150–300/year", EV "$400+/year" | | | [H] Learn, code-signing options, ms.date 2026-08-29 |

**Automation caveat [M]:**
- Certum SimplySign is interactive: you log into SimplySign Desktop with a code from the mobile app.
- Headless CI signing works only through community tools that store the TOTP seed (e.g., the `ssign` crate), which effectively turns the seed into a long-lived secret.
- SSL.com eSigner and Artifact Signing have documented CI integrations.
- For a solo developer who builds releases on the Windows gaming PC, interactive signing is acceptable.

**Privacy note:** an individual certificate shows **your legal name**, plus locality and country, as the publisher in UAC and SmartScreen dialogs [H, Artifact Signing subject preview; standard for OV/IV].

### 1.5 SignPath Foundation (free, OSS only)

From signpath.org/terms (undated draft) [H]:
- **License:** "OSI-approved Open Source license **without commercial dual-licensing** for all components."
- **Proprietary code:** no proprietary components, though system libraries are OK.
- **Release state:** actively maintained and already released.
- **Reputation:** for executables, "a certain verifiable reputation" is required. This is a chicken-and-egg problem for a brand-new app.
- **Build:** binaries must be built "from source code in a verifiable way" through automated CI. A manual approval is needed per release, and MFA is required for every team member.
- **Disclosure:** a "Code signing policy" section must appear on the project page.
- **Publisher:** **"SignPath Foundation is the publisher of the OSS project."** The certificate shows SignPath Foundation, not Kairo.
- **Revocation:** the Foundation can revoke "immediately or retroactively".

Microsoft's own code-signing page lists SignPath Foundation as the OSS option [H]. Fit for Kairo: only if Kairo is fully OSI-licensed (MIT/Apache/GPL) with no closed parts. Note that LGPL FFmpeg DLLs and MIT GPU SDK headers are open source, so they don't disqualify it.

### 1.6 What and how to sign (practical)

- **What to sign:**
  - every PE file you ship: `Kairo.exe`, the engine exe, the installer, the uninstaller, **and the FFmpeg DLLs** (BtbN builds are unsigned, and you are redistributing them);
  - MSIX packages, if you sideload them.
- **How:** use SHA-256 (`/fd sha256`) with an RFC 3161 timestamp (`/tr … /td sha256`). Timestamping is mandatory with 72-hour Artifact Signing certificates and keeps OV signatures valid after expiry [H].
- **Avoid drivers.** Kernel drivers (virtual audio, virtual camera) need the Hardware Developer Program, which requires an **EV certificate** and attestation signing [H, Learn, ms.date 2025-05-29]. Kairo's audio plan uses process loopback (report 02), so no driver is needed.
- **Hotkeys:** prefer `RegisterHotKey` to low-level keyboard hooks (`WH_KEYBOARD_LL`). Hooks look like keylogging to heuristics [L, inference].

---

## 2. Microsoft Store

### 2.1 Developer accounts (changed in 2025–2026)

- **Fees:** "With the new onboarding experience, there are **no registration fees for either account type**" (Learn, open a developer account, updated 2026-07-17) [H].
  - The Sept 2025 Windows blog announced free *individual* registration in "nearly 200 markets" with no credit card [H].
  - Press in 2025 still quoted $99 for companies. The current Learn page supersedes that.
- **Individual flow:** personal Microsoft account, then government ID plus selfie [H].
- **Company flow:** D-U-N-S number (recommended) or official business documents, plus a **work email on the company domain**. Manual review takes 2–5 business days [H].
- **Which type:** Individual is for distribution "**not** in relation to their business, trade, or profession". Company covers "independent developers and freelancers whose distribution… is in relation to their business, trade, or profession" [H].
  - Policy 10.14 also requires a company account "if a reasonable consumer would interpret your application or publisher name to be that of a business entity" [H, Store Policies v7.20].
- **"Changing a developer account from Individual to Company is not supported…** you'll need to create a new Company developer account." [H]
  - I found no official app-transfer procedure, so ask Partner Center support [L].
  - This matters for a "free now, monetize later" plan. See section 9.

### 2.2 MSIX vs EXE/MSI submissions

| | **MSIX** | **EXE / MSI (policy 10.2.9)** |
|---|---|---|
| Who signs | **Microsoft re-signs after certification. Free.** | You sign, with a cert chaining to a **Trusted Root Program CA**. Self-signed is rejected. |
| Hosting | Store CDN, free | **Your HTTPS URL, versioned. The binary at that URL must never change.** |
| Updates | **Store auto-updates.** In-app control via `StoreContext.GetAppAndOptionalStorePackageUpdatesAsync` / `RequestDownloadAndInstallStorePackageUpdatesAsync` (Win10 1607+). Silent `TrySilentDownloadAndInstallStorePackageUpdatesAsync` (Win10 1803+) works only if the user has "Update apps automatically" on and isn't on a metered network. | The **Store doesn't update you**; your own updater does. Each new version needs a new versioned URL in Partner Center [H; 2021 press for "apps update themselves" M] |
| Install rules | Per-user package; clean uninstall | **Silent install required** (UAC allowed), **standalone/offline** (no web stub), **non-gaming products only**. Download time and install success rate are monitored (10.4.4) |
| SmartScreen | Never (Store-installed) | "No SmartScreen prompts during Store install" |
| Tauri | Tauri only generates EXE/MSI. MSIX needs extra tooling (winapp CLI, see 2.3) | Supported. Must use **WebView2 `offlineInstaller`** (+~127 MB), silent args `/S` (NSIS) or `/quiet` (MSI), and `bundle.publisher` ≠ product name [H, Tauri docs] |
| Betas | **Private audience** (hidden listing for known user groups). Testers need a Microsoft account. Usable only before a public release, and access can't be revoked after download [M, secondary] | Not verified for EXE/MSI |
| Crash data | Partner Center **Health report**: crashes/hangs with stack traces/CABs where available, plus 2026 Anomaly Alerts [M] | Unclear whether Win32 EXE apps get usable data. Developers report "Unknown" failures [M] |

Sources: Learn MSI/EXE requirements (updated 2026-08-24), Store Policies v7.20 (ms.date 2026-09-14), Learn code-signing options (2026-08-29), Learn self-install package updates, Tauri "Microsoft Store" docs.

**Store Web Installer:**
- Your website can show a "Get it from Microsoft" badge with launch mode "Direct".
- The Feb 2026 update added **auto-open after install for Win32 apps** [H, Windows Developer Blog 2026-02-11].
- Whether the stub itself avoids SmartScreen is not documented [L].

### 2.3 Getting a Tauri/Rust app into MSIX

- **winapp CLI** (Microsoft, open source, **public preview**, Learn ms.date 2026-10-08) lists **Tauri and Rust** as supported frameworks.
  - `winapp pack` "builds your project, packages it as MSIX, and signs it in one step".
  - `winapp az-sign` signs with Artifact Signing.
  - There is a Tauri sample and a **Claude Code plugin** (`claude plugin install winappcli@winappcli`) [H].
  - Being a preview, commands may change [H].
- **Plan for two build flavors:**
  - **Store MSIX:** Store updates; startup task in the manifest.
  - **Direct download:** NSIS/Velopack with your own updater and an HKCU Run key.
  - The reasons are MSIX runtime rules (2.4).

### 2.4 Capture-app-specific Store and MSIX constraints

| Topic | Rule | Tag |
|---|---|---|
| **runFullTrust** | A restricted capability, but standard for packaged desktop apps. "A Medium IL app needs to declare the runFullTrust restricted capability." Approval is requested in the submission and normally not repeated for updates. | [H] capability declarations |
| allowElevation | "Subject to approval under strict criteria", and you must email reportapp@microsoft.com first. **Design Kairo to run unelevated.** | [H] |
| **Start at login (MSIX)** | Use the `windows.startupTask` extension. A packaged desktop app may set `Enabled="true"`, but "the user must either launch the app at least once, or… enable startup functionality… in Settings". The user can disable it in Task Manager or Settings (`DisabledByUser`), and the app can't override that. | [H] StartupTask class |
| HKCU Run key under MSIX | "All writes under HKCU are copied on write to a private per-user, per-app location", so a classic `HKCU\…\Run` entry written from MSIX doesn't register real autostart. The Tauri `autostart` plugin won't work in the MSIX flavor. | [H] Learn, packaged desktop apps (ms.date 2026-09-30) |
| Settings changes | 10.2.8: "must obtain user consent to change any user's Windows settings". Make "Start with Windows" an explicit onboarding choice. | [H] |
| **Global hotkeys** | No Store policy restriction. `RegisterHotKey` works for full-trust apps. Conflicts with other apps are a runtime issue (report 05). | [M] |
| **Screen capture** | If packaged: `graphicsCapture` (picker), `graphicsCaptureProgrammatic` (create a `GraphicsCaptureItem` from a WindowId/DisplayId) and `graphicsCaptureWithoutBorder` (`IsBorderRequired`). These are uap6/uap11 **general-use, not restricted**, capabilities. | [H] |
| Microphone | A privacy-sensitive capability. Users can deny it in Windows privacy settings, so the app must handle denial. | [H] |
| Drivers / services | 10.2.4: "dependency on non-Microsoft provided drivers or NT services is not allowed but may be considered case by case". | [H] |
| Dynamic code | 10.2.2: no downloading scripts that change described functionality. This matters for plugin or auto-highlight model downloads. | [H] |
| Privacy policy | 10.5.1: "Desktop Bridge and Win32 products" **must always** have a privacy policy URL. | [H] |
| Age rating | 11.11: IARC questionnaire at submission; keep it current. | [H] |
| UGC (future cloud) | 11.12: in-product ToS/content guidelines, an in-product report mechanism, and respect for parental settings. | [H] |

---

## 3. Installers and updaters

| Tool (version, date) | Per-user, no admin | Delta updates | Silent / background updates | Update integrity | Cost / license | Fit for Kairo |
|---|---|---|---|---|---|---|
| **Tauri NSIS + `tauri-plugin-updater`** (tauri 2.12.4, plugin 2.14.0, both 2026-10-14) | **Yes, by default** ("current user only", `%LOCALAPPDATA%`). `perMachine`/`both` need admin | **No.** Docs describe full bundles only; the only delta issue is AppImage | `installMode`: `passive` (default, small progress window), `basicUi`, `quiet` (per-user only). "The app exits during installation" on Windows. Hooks `NSIS_HOOK_PREINSTALL/POSTINSTALL` can stop and restart the engine | **Mandatory** update signature: `TAURI_SIGNING_PRIVATE_KEY` + `pubkey`; "This cannot be disabled" | MIT/Apache; free | **Default choice.** Lowest friction with the Tauri UI |
| **Velopack** (1.2.161, 2026-09-29; 1.3.x prerelease) | **Yes**: `Setup.exe` → `%LocalAppData%\{packId}`. MSI PerUser needs no elevation | **Yes**: per-file zstd patches; chains deltas or falls back to full by heuristic; 2 GB per-file limit | `UpdateManager` API in Rust/C#/JS/C++/Python; apply-on-exit pattern | Feed carries SHA1/SHA256 per asset. I couldn't confirm a separate feed signature, so rely on HTTPS + Authenticode [L] | **MIT**, free. Optional MSI via Velopack's own WiX v7 fork "avoids the OSMF fee" | Strong alternative if large rarely-changing DLLs (FFmpeg) make full downloads painful. It replaces the Tauri bundler for the direct-download flavor |
| **MSIX + App Installer** (sideload) | Yes (per-user packages) | **Yes**: only changed **64 KB blocks** are downloaded | `.appinstaller` 2021 schema: `HoursBetweenUpdateChecks`, `ShowPrompt`, `UpdateBlocksActivation`. Visual Studio emits the 2017/2 schema, which silently ignores them | MSIX signature (needs your own trusted cert outside the Store) | Free | Weak outside the Store: **`ms-appinstaller:` links disabled by default since 2023-12** (App Installer 1.21.3421.0); users must download the `.appinstaller` file. Several MSIX features are Windows 11-only |
| **MSIX via Store** | Yes | Yes (block map) | Store auto-update + `StoreContext` APIs | Microsoft re-signs | Free | **Best public channel** (no SmartScreen) |
| **electron-builder / electron-updater** (26.18.0 / 6.8.11, 2026-10-20; v27 in alpha) | Yes: NSIS `oneClick: true`, `perMachine: false` by default | **Yes**: NSIS blockmap `differentialPackage: true` by default | Installs on app quit by default (`autoInstallOnAppQuit`). The v27 docs add `autoInstallEvent` with an `onNextLaunch` option | Authenticode verification on Windows; **v27 adds Ed25519-signed update manifests** | MIT | Only relevant if the UI were Electron (report 05 advises against) |
| **Inno Setup** (7.1.0, 2026-08-12) | Yes (non-admin installs supported) | No built-in updater | n/a | n/a | "We request that all commercial users… purchase licenses" (revenue > $5,000). "Not strictly required" | Not needed |
| **NSIS** (3.13, 2026-09-27) | Yes | No built-in updater | n/a | n/a | zlib-style, free | Used under the hood by Tauri |
| **WiX Toolset** (v7) | Per-user MSI is possible | No | n/a | n/a | **OSMF:** organizations with >$10,000 annual revenue "are required to sponsor the wixtoolset GitHub organization". v7 enforces EULA acceptance (`-acceptEula wix7`) | Avoid unless you need MSI for enterprises |

Sources: Tauri updater and Windows-installer docs; crates.io; Velopack docs and GitHub releases; Learn MSIX overview and distribution-feature-status (last reviewed Aug 2026); electron-builder repo docs and `nsisOptions.ts`; jrsoftware.org; nsis.sourceforge.io; docs.firegiant.com.

**Update-time rules specific to Kairo [M, design guidance]:**
- Never apply an update while a game is running or a clip is being saved. Download in the background and install on exit or idle, or when the user clicks "Restart to update".
- Stop the engine in a pre-install hook and restart it after. Tauri `NSIS_HOOK_PREINSTALL/POSTINSTALL` does this (report 05), as do Velopack hooks.
- Hosting: **GitHub Releases** has no bandwidth limit, files must be under 2 GiB, and a release can hold up to 1,000 assets [H].
  - A private repo can't serve anonymous updates. Keep sources private if you like, but publish releases from a public "kairo-releases" repo or object storage [L, inference].

---

## 4. Crash reporting and telemetry

### 4.1 Options

| Option | What you get | Price / limits | Tag |
|---|---|---|---|
| **Sentry (SaaS)** | Native crash ingestion (minidumps), symbolication from uploaded PDBs, release health | **Developer: free, 5k errors, 1 user, 30-day lookback, 1 GB attachments.** **Team: $26/month annual or $29 monthly**, 50k errors, unlimited users, 90-day. Business $80/$89 | [H] sentry.io/pricing today |
| Sentry data region | **EU (Frankfurt)** chosen at org creation, available on all plans per Sentry support. Some data types may still sit in the US | | [M] |
| Sentry for OSS | "Sentry for Good" sponsorship for open-source/non-profit orgs, at Sentry's discretion | | [M] |
| **sentry-native** (0.17.4, 2026-10-16, MIT) | Backends: **crashpad** (default on Windows; ship `crashpad_handler.exe`), **native** (`sentry-crash` + `sentry-wer.dll` for **fast-fail** crashes), breakpad, inproc. Optional WER integration | | [H] GitHub README |
| **Rust engine** | `sentry` 0.49.3 for panics and errors. `sentry-rust-minidump` 0.18.1 (out-of-process minidumps via EmbarkStudios `minidumper` 0.11.0 / `crash-handler` 0.8.1) for access violations inside COM/FFmpeg | | [H] crates.io versions; [M] integration claims |
| Crashpad / Breakpad (self-run) | Google's crash handlers write minidumps. You then need your own collector and symbol processing. Not worth it solo | | [M] |
| **Windows Error Reporting** | Always on for crashes. **LocalDumps** (`HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps`, default `%LOCALAPPDATA%\CrashDumps`, DumpCount 10, DumpType 1) is useful for beta testers but **needs admin to enable**. `RegisterApplicationRestart` restarts the app after crash/hang **only with user consent** and only if it ran **≥60 s**. Non-Store WER data access historically went through the hardware dashboard; don't rely on it | | [H] Learn; [L] WER portal access |
| Store **Health report** | Crashes and hangs for Store-distributed apps, with Anomaly Alerts (Feb 2026) | free | [M] |

**Symbol hygiene:** upload PDBs to Sentry from CI with `sentry-cli debug-files upload`. Never ship PDBs to users [M, standard practice].

### 4.2 Privacy of crash data

- Minidumps can contain "environment variables, local pathnames or maybe even in-memory representations of input fields, including passwords".
- **Sentry by default "only uses minidump files to create events and immediately drops them".** "Store Minidumps As Attachments" is **off by default**, and **Sentry doesn't scrub attachments** [H/M, Sentry docs].
- For Kairo, frame buffers, chat overlays and file paths with the Windows username can end up in memory. Keep minidump storage off and scrub paths in `before_send`. Never attach screenshots or clips to crash reports automatically.

### 4.3 Is opt-in required? (EU)

- **ePrivacy Art. 5(3)** requires consent for storing or accessing information on a user's device, unless it is strictly necessary for a service the user explicitly requested, or for transmission.
- **EDPB Guidelines 2/2023, v2.0 (adopted 2024-10-07), para 33:** "That is equally the case when the accessing entity distributes software on the terminal equipment of the user that is stored and will then proactively call an Application Programming Interface ('API') endpoint over the network." [H, quoted from the PDF]
- **My reading [M]:** crash reports and analytics sent by Kairo fall under 5(3) and aren't "strictly necessary" for recording clips. Consent is required: an explicit opt-in toggle in onboarding plus a per-crash "Send report?" prompt.
- **Update checks** are a judgment call. Security updates are arguably strictly necessary. Keep the payload minimal (version, OS build, arch, no ID), disclose it, and let users turn automatic checks off [L].
- **Keep telemetry optional.** A free app that requires personal data as a condition of use starts to look like a contract paid for with data. That has consumer-law consequences (Digital Content Directive 2019/770 Art. 3(1)). It is also a CRA "commercial activity" indicator (section 5.5) [M].
- **Lightweight analytics if wanted:** Aptabase is free for 20,000 events/month, $10/month for 200k, with EU or US data residency [H pricing]. Or simply none during beta.

### 4.4 Privacy policy contents

Required by the Store for every Win32 app (10.5.1), and by GDPR Art. 13 whenever you collect anything [H].

- **Local-only v1 (no accounts, no cloud):**
  1. Identity and contact of the controller (you). Some states also require a legal notice (*Impressum*), e.g., Germany [M].
  2. A plain statement that recordings, screenshots and audio **stay on the device**, and that Kairo never uploads them.
  3. What *does* leave the device, and why:
     - update checks (IP address, app/OS version);
     - opt-in crash reports (stack traces, device info, possibly fragments of memory);
     - opt-in analytics events.
  4. Legal basis for each item (consent for crash/analytics; legitimate interest or necessity for update checks) and how to withdraw consent.
  5. Processors and recipients (Sentry; the update host, e.g. GitHub/CDN, which logs IPs), international transfers and safeguards (EU-US DPF or SCCs), and retention periods.
  6. Rights: access, erasure, objection, the right to complain to a supervisory authority. Plus children (see 5.4), and how changes are announced.
- **Cloud features add:**
  - account data;
  - uploaded clips (which can contain **other people's voices and images**);
  - sharing visibility;
  - moderation and DSA notice-and-action;
  - DMCA contact;
  - retention and deletion of clips;
  - age limits;
  - storage/CDN processors;
  - possibly an EU or UK representative if you are not established there.
- Report 06 covers the hosting duties in detail.

---

## 5. Legal

### 5.1 Publisher policies on gameplay clips

| Publisher | Policy (date) | Videos / screenshots | Monetization | Key restrictions | Tag |
|---|---|---|---|---|---|
| **Microsoft / Xbox Game Studios** | Game Content Usage Rules (last updated **Jan 2015**) | Personal, non-commercial use; derivative works OK; YouTube/Twitch OK | Ad revenue via YouTube/Twitch partner programs OK. **No subscription or paywall** | **"In an app that you sell in an app store"**: not allowed. **Promoting a commercial venture needs permission.** Must include a "not endorsed by Microsoft" notice **and a link to the Rules** | [H] |
| **Riot** | "Legal Jibber Jabber" (Aug 2018) | Allowed for fan projects | Passive ads OK. Stream donations/subs OK if non-subs can still watch | **No paywalls or crowdfunding** without a license. **"Prohibit the use of our IP in games and apps"**; store apps need a license or a valid Riot API key. Disclaimer text required | [H] |
| **Valve** | Video policy (undated) | Encouraged | "Free to monetize… via the YouTube partner program and similar programs" | "Must be non-commercial". **Can't charge viewers**, can't sell or license videos. Third-party music needs the owner's permission | [H] |
| **Epic** | Fan Content Policy | Allowed | Only individual fans may monetize web videos with ads | No commercial objective. Epic can withdraw permission at any time. Fortnite: set **Licensed Audio → Mute All** before streaming | [M] (page 403; via search and Epic help center) |
| **EA** | EA content policy (help.ea.com) | Personal uses and projects | Partner programs and passive ads OK | **No paywalls incl. Patreon**, no selling, no pre-release content | [M] |
| **Activision** | No current public creator policy found | n/v | n/v | Blizzard's older video policy allowed free-to-view videos with partner fees | [L] |
| **Nintendo** | Game Content Guidelines (**2024-09-02**) | Requires **creative input or commentary**, **except** content captured "using Nintendo system features" | Only on listed platforms: YouTube, Twitch, X, Facebook, Instagram, Niconico, TwitCasting, 17LIVE | **No selling.** Only post-release content. No emulator or circumvention content. Nintendo may object to content and ban creators | [H] |

**What this means for Kairo:**
- **Users** are responsible for what they post. Kairo's ToS and first-share dialog should say so.
- **Kairo's own marketing:**
  - Store screenshots, trailers, website and ads are commercial use.
  - Use your own UI shots, open-licensed games, or footage you have **written permission** for.
  - Avoid Xbox/Activision and Riot footage in particular.
- **Future cloud features:**
  - Don't build monetization that **paywalls viewing of game clips** (e.g., "subscribers-only clips"). That collides with Microsoft, Riot, Valve and EA rules.
  - Charging for *storage/features* is a different question for a lawyer [L].
- **Console clips via capture card:** raw, uncommented Nintendo footage is outside Nintendo's guidelines. Only system-captured footage is exempt.
- **Game integrations** (e.g., auto-highlights from game APIs) carry their own API terms (Riot API, etc.). See report 03.

### 5.2 Copyrighted music and DMCA

- Game licenses for music usually **don't extend to streams or uploads**. Fortnite added a Licensed Audio mute for exactly this reason [M].
- In 2020, record labels flooded Twitch with notices: Twitch had "fewer than 50" music DMCA notices a year before May 2020, then mass notices mostly targeting **old clips**, and it deleted clips en masse [M, Tubefilter Nov 2020].
- **Product mitigations (v1, local):**
  - separate audio tracks (game / mic / voice-chat app) so music can be muted when editing;
  - a share-time reminder about in-game music.
- **When Kairo hosts clips (later):**
  - US **DMCA §512** designated agent: **$6 per designation, expires after 3 years** [H].
  - Repeat-infringer policy; EU DSA notice-and-action (Arts. 11–18 for all hosts); UK Online Safety Act illegal-harms risk assessment "regardless of size" [M].
  - The DSM Directive Art. 17 regime for content-sharing platforms needs counsel before public feeds.
  - Report 06 covers hosting duties and costs.

### 5.3 Recording other people's voice chat

- **EU criminal law varies by country.** In Germany, §201 StGB punishes recording the non-public spoken word without consent, even by a participant: up to 3 years or a fine [M, law-firm summary]. Other member states differ. Check the developer's own country.
- **GDPR:** purely personal or household recordings are exempt. **Publishing** a recording on a platform with unrestricted access brings it back into GDPR (CJEU C-345/17 *Buivids*, 2019; cf. C-101/01 *Lindqvist*) [M].
- **US:** federal one-party consent, but California Penal Code §632 requires **all-party** consent for confidential communications, with civil exposure (§637.2) [M].
- **Product defaults that reduce harm (recommended):**
  1. The microphone track is the user's own mic only.
  2. Voice-chat apps (Discord, in-game VOIP processes) go on a **separate, default-off** track via process loopback include/exclude. See report 02, which documents the API's build floor (20348; 19041 in practice).
  3. A one-time notice: "Recording others may require their consent; check local law."
  4. At share/upload time, offer "remove voice-chat track".

### 5.4 Minors

- **Local-only v1 with no accounts** collects almost nothing, so minors are mainly a Store rating (IARC) question [H].
- **If telemetry relies on consent and Kairo targets teens:**
  - GDPR Art. 8 sets the age of digital consent at **16 by default**. States may lower it to 13 (e.g., Germany 16, France 15) [M, secondary table].
  - Keep telemetry off by default and don't market to children.
- **Cloud accounts:** the amended **COPPA** rule took effect 2025-06-23, with compliance required by **2026-04-22** [M, Federal Register notice as quoted in summaries]. A 13+ floor globally, and 16+ for public social features in the EU, is the practical posture (report 06) [M].

### 5.5 EU product rules that reach a software developer

- **Cyber Resilience Act (Reg. 2024/2847):**
  - **Reporting** of actively exploited vulnerabilities and severe incidents via **ENISA's Single Reporting Platform since 2026-09-11** [H].
  - Deadlines: early warning **24 h**, notification **72 h**, final report **14 days** after a fix [M, DLA Piper].
  - **Full obligations from 2027-12-11** [H].
  - They include a declared **support period**, where 5 years is "the minimum legal requirement" if expected use is that long [M, Osborne Clarke on the guidance].
  - From my reading of the regulation (not re-read today) [M]: security-by-design, vulnerability handling, an SBOM and technical documentation, and CE marking.
  - The Commission's guidance was **adopted 2026-07-27** and is non-binding [M].
  - **Scope trigger** (recital 15): supply "in the course of a commercial activity", including "charging a price for technical support services", "an intention to monetise… by providing a software platform", "requiring as a condition for use the processing of personal data for reasons other than exclusively for improving the security, compatibility or interoperability", or donations exceeding costs. Non-monetized FOSS isn't commercial (recital 18) [M, recital text via mirror].
  - **Kairo:** a free, non-monetized, open-source hobby release is likely out of scope. Any paid tier, or required data collection, brings it in. A screen recorder isn't in the "important products" annexes, so self-assessment applies [L].
- **Product Liability Directive (2024/2853):** software is a product. It applies to products placed on the market **after 2026-12-09**, and transposition is due the same day [M].
- **Windows 10 context:** consumer ESU was reportedly extended to **2027-10-12** (Help Net Security, 2026-06-26) [M]. Win10 22H2 users will be around through 2027.

---

## 6. Open-source licensing options

### 6.1 Dependency constraints

| Dependency | License | Constraint | Tag |
|---|---|---|---|
| **libobs** (OBS 32.2.2, 2026-08-14) | Repo GPL-2.0. libobs file headers: "version 2 of the License, or (at your option) any later version" | Linking it makes the distributed combination GPL. A separate process talking over pipes is *usually* a separate program per the FSF, unless the communication is "intimate" (complex internal data structures) | [H] license; [M] FSF FAQ |
| **FFmpeg** | LGPL-2.1-or-later. **GPL** if built `--enable-gpl` (e.g., libx264); `--enable-nonfree` is not redistributable | LGPL checklist: dynamic DLLs; ship FFmpeg source; don't obfuscate DLL names; state LGPL use in the EULA; **"remove any ban on reverse engineering"**; no GPL libs | [H] ffmpeg.org/legal |
| AMD AMF SDK | MIT | **"AMD does not provide a license… to… AVC/H.264; HEVC/H.265; AAC… you will pay any royalties due"** | [H] |
| Intel VPL, NVIDIA nv-codec-headers | MIT | None for licensing (report 05) | [H via report 05] |
| Codec patents | Via LA AVC pool | First **100,000 units/year royalty-free**, then $0.20/unit up to 5M and $0.10 above (PC software outside the OS). How this applies to free software is unclear; get counsel before monetizing or at scale | [M] Via LA summary |
| Tauri, Velopack, sentry-native | MIT/Apache, MIT, MIT | Compatible with anything | [H] |
| WiX v7 / Inno Setup | OSMF / commercial-license request | Fees at revenue thresholds (section 3) | [H] |

### 6.2 License choices for Kairo

| Choice | Pros | Cons | Signing and other effects |
|---|---|---|---|
| **MIT or Apache-2.0** (Apache adds a patent grant) | Maximum trust for a privacy-sensitive capture app; contributions; easy for an AI-assisted solo dev | Anyone, including Medal or Overwolf-style competitors, can fork commercially; **can't link libobs** | SignPath Foundation eligible (if no proprietary parts); Certum OSS cert (only while non-commercial); Sentry sponsorship possible |
| **GPL-3.0 (or GPL-2.0+)** | **Required if you link libobs**; forks stay open; you can still sell binaries and services | Some contributors and companies avoid GPL; relicensing later needs a **CLA** (and is impossible for libobs code) | Same OSS signing options |
| AGPL-3.0 (for a future cloud backend) | Protects the hosted service from closed forks | Same as GPL | n/a |
| **Fair source** (FSL-1.1-MIT/ALv2: each version becomes MIT/Apache **after 2 years**; "anything… except undermine its producer"), BUSL | Source visible for trust; blocks competing commercial forks | **Not OSI**: no SignPath Foundation, no "open source" claim, can't combine with GPL libobs | Pay for OV signing |
| Closed source | Full control | Less trust; no OSS perks | Pay for OV signing |

**Recommendation [M]:**
- Decide the license **before** accepting outside code. If you want any future relicensing, use a CLA or DCO.
- If report 05's Rust engine (no libobs) holds, two options fit:
  - **MIT/Apache-2.0** for maximum trust and free OSS signing;
  - **FSL-1.1-ALv2**, if protecting a future business matters more than OSS perks.
- Use GPL only if you adopt libobs.

---

## 7. Shipping checklist

### 7.1 Before the private beta

1. **Decide the identity.** Legal name or company, the **Store account type** (can't convert later), and the publisher display name (10.14).
2. **Buy signing:** Certum Standard Cloud (~€209) for an EU individual, Artifact Signing ($9.99/month) for an EU company, US or CA. Allow 1–20 business days for validation.
3. **Sign** every PE file plus the installer and uninstaller (SHA-256 + RFC 3161 timestamp). Keep the certificate subject stable.
4. **Installer:** Tauri NSIS `currentUser` (or Velopack) with no admin, a clean uninstall, and no bundled third-party offers (10.2.3).
5. **Updater:** generate and **back up the Tauri updater key** offline (losing it strands users). Host releases on a public repo or bucket. Never update mid-game. Engine stop/start hooks.
6. **Crash reporting:** Sentry Developer plan (EU region), PDB upload in CI, minidump storage off, path scrubbing, **opt-in** consent screen, a "Send report?" prompt.
7. **Autostart and hotkeys:** opt-in "Start with Windows" (HKCU Run, which respects the Startup Apps toggle), `RegisterHotKey`, conflict UI.
8. **Audio defaults:** own mic only; voice-chat apps on a separate default-off track; a consent notice.
9. **Docs:** a privacy policy (local-only version), a simple EULA/ToS including the **LGPL notice and no reverse-engineering ban**, a third-party notices file with **FFmpeg source and configure line**, and a "testers will see SmartScreen" note.
10. **AV:** submit each release to Defender as "software developer" if flagged. Test on a Windows 11 PC with **Smart App Control on**.

### 7.2 Before the free public release

11. **Store MSIX flavor** via winapp CLI (preview):
    - `runFullTrust` justification;
    - `windows.startupTask`;
    - no self-updater, because the Store updates the app;
    - `graphicsCapture*` and microphone capabilities as needed;
    - IARC questionnaire, privacy policy URL, Store listing assets **without third-party game footage unless licensed**.
12. **Direct-download flavor** stays signed with the same identity. Consider winget/Chocolatey listings as SmartScreen-free alternatives [M].
13. A security contact (e.g., `security.txt`) and a written vulnerability-handling process. Cheap now, and mandatory under the CRA once commercial.
14. Optional analytics: opt-in only, no persistent unique ID, EU hosting.
15. Name clearance for "Kairo" (EUIPO/USPTO/Store search) before spending on branding [L; not researched here].

### 7.3 Before monetization or cloud sharing

16. Company formation, and a new **company** Store account if needed (2.1). Re-plan signing: switching to Artifact Signing changes the publisher identity, so SmartScreen publisher reputation restarts (the Store isn't affected).
17. **CRA readiness:** support period, SBOM, ENISA reporting runbook (24 h / 72 h), CE self-assessment by 2027-12-11. Also PLD exposure.
18. Codec patent review (AMF notice; Via LA AVC/HEVC), plus VAT and payments via a merchant of record.
19. Cloud: DMCA agent ($6/3 yrs), DSA notice-and-action, UK OSA risk assessment, COPPA 13+ gate, ToS/UGC rules (Store 11.12), moderation and CSAM runbook (report 06).

---

## 8. Cheapest credible path and costs (solo developer)

| Item | Store-only ($0) | **Recommended: EU individual** | EU company | US/CA individual |
|---|---|---|---|---|
| Store account | free | free (Individual, *or* create Company from day one if monetization is likely) | free (D-U-N-S/docs + domain email) | free |
| Code signing | none (MSIX re-signed by Store) | **Certum Standard Code Signing in the Cloud, from €209** (individual OV; interactive SimplySign) | **Artifact Signing Basic $9.99/month ≈ $120/yr** + paid Azure sub (pay-as-you-go) | Artifact Signing individual, $9.99/month |
| OSS alternative | | Certum OSS €49 (non-commercial only) or SignPath Foundation (free; Foundation as publisher; reputation needed) | same | same |
| Updates hosting | Store CDN | GitHub Releases (free; no bandwidth cap; under 2 GiB per file) | same | same |
| Crash reporting | Store Health report | Sentry Developer, free (5k errors/month, 1 user) → Team $26/month when needed | same | same |
| Analytics | none | none in beta; Aptabase free tier (20k events/month) later | same | same |
| **Year-1 cash** | **$0** | **≈ €209** (+ domain) | **≈ $120** (+ company costs) | **≈ $120** |
| Main drawback | MSIX-only; slow certification per beta build; no direct download | Name on certificate; manual signing; annual re-validation | Company overhead; possible history requirement [L] | n/a |

**Why not Store-only for the beta?**
- Store certification adds latency to every beta build.
- Private-audience testers need Microsoft accounts.
- MSIX changes autostart and update behavior.

Use the Store as the **public** channel and signed direct downloads for the fast-moving beta.

---

## 9. Questions the user must answer

1. Where are you legally resident? Will you register a company (and when)? This decides Artifact Signing eligibility, the Store account type (not convertible), CRA exposure and the name shown on the certificate.
2. Will Kairo be open source? Under which license (MIT/Apache, GPL, or fair source like FSL)? Will you ever use libobs?
3. Which public channel comes first: Microsoft Store MSIX (free, no SmartScreen, Store updates), signed direct download (≈€209/yr), or both?
4. What is your budget ceiling for year 1: €0, about €50 (OSS cert), about €209 (OV), or about $120 + company?
5. Crash reports and analytics: opt-in crash reports only, or also product analytics? Is Sentry (US company, EU region) acceptable?
6. Beta logistics: how many testers? Do they have Microsoft accounts? Will they accept a SmartScreen "Run anyway" on first install?
7. Audio defaults: should Kairo ever record other people's voice chat by default? Separate tracks?
8. Monetization model and timing (paid features, subscription, cloud storage)? This triggers CRA, codec-patent and Store account-type questions.
9. Cloud sharing: private/unlisted links only, or public profiles and feeds? Minimum user age?
10. Is the name "Kairo" cleared, and what publisher display name do you want on the Store and the certificate?

---

## 10. Unverified or conflicting items

- **Artifact Signing and the Store EXE route:** CAs are being removed from CCADB and the Trusted Root Program, while Store policy 10.2.9 requires a Trusted Root Program chain. This is unresolved [L].
- **Artifact Signing org history:** the 3-year history requirement (2025 forum) is not in current docs [L].
- **Velopack:** its "EV for instant reputation" advice contradicts Microsoft's 2026 docs. Trust Microsoft [H].
- **Smart App Control:** the October 2026 KB5127648 greyed-out toggle comes from one French report [L].
- **Certum:** whether "from €209" includes the first-year cloud subscription and VAT for EU consumers is unverified [M].
- **Sectigo:** individual availability and reseller prices are unverified [L].
- **Epic and EA:** policy pages could not be fetched directly (403/timeout). The text came via search snippets and the Epic help center [M].
- **EU law:** CRA recital wording, DSA article structure, GDPR Art. 8 ages, §201 StGB and *Buivids* come from mirrors and summaries, not EUR-Lex/CURIA [M].
- **Codec royalties:** how the Via LA "units" definition treats free software is unclear [L].
- **Win32 EXE apps:** whether they get useful Partner Center Health data is unclear [L].

---

## Sources

**Code signing and SmartScreen**
- https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart (ms.date 2026-05-21, updated 2026-10-08)
- https://learn.microsoft.com/en-us/azure/artifact-signing/faq (ms.date 2026-05-14, updated 2026-10-20)
- https://learn.microsoft.com/en-us/azure/artifact-signing/concept-certificate-management (ms.date 2026-09-30)
- https://prices.azure.com/api/retail/prices?$filter=contains(productName,'Signing') (queried 2026-10-21; rows effective 2024-06-01)
- https://azure.microsoft.com/en-us/pricing/details/artifact-signing/ (prices not rendered, 2026-10-21)
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation (ms.date 2026-05-04, updated 2026-08-17)
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/distribution-feature-status (last reviewed Aug 2026)
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options (ms.date 2026-08-29)
- https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control (ms.date 2026-09-25)
- https://blog.topedia.com/2026/04/smart-app-control-in-windows-11-can-now-be-re-enabled-without-reinstalling/ (Apr 2026)
- https://www.it-connect.fr/windows-11-kb5127648-smart-app-control-grise/ (2026-10-18, via search only)
- https://cabforum.org/2025/11/17/ballot-csc-31-maximum-validity-reduction (2025-11-17)
- https://shop.certum.eu/code-signing.html (read 2026-10-21)
- https://shop.certum.eu/open-source-code-signing-on-simplysign.html (read 2026-10-21)
- https://shop.certum.eu/open-source-code-signing.html (read 2026-10-21)
- https://support.certum.eu/en/code-signing-required-documents/ (read 2026-10-21)
- https://www.ssl.com/certificates/code-signing/buy/ (read 2026-10-21)
- https://www.ssl.com/certificates/ev-code-signing/buy/ (read 2026-10-21)
- https://ssl.com/guide/esigner-pricing-for-code-signing/ (updated 2026-03-25)
- https://www.sectigo.com/ssl-certificates-tls/code-signing (read 2026-10-21)
- https://signpath.org/terms (undated draft)
- https://textslashplain.com/2024/11/15/best-practices-for-smartscreen-apprep/ (2024-11-15)
- https://markdownmonster.west-wind.com/blog/posts/2026/Jun/01/Windows-Protected-your-PC-Dealing-with-Windows-SmartScreen-on-Installation (2026-06-01)
- https://learn.microsoft.com/en-us/defender-xdr/submission-guide (updated 2026-09-10)
- https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-cert-manage (ms.date 2025-05-29)
- https://learn.microsoft.com/en-us/answers/questions/5532414/trusted-signing-identity-verification-failed-provi (2025-08-24, volunteer moderator)
- https://learn.microsoft.com/en-us/answers/questions/5915697/can-an-irish-sole-trader-(non-incorporated)-qualif (2026-06-09, AI answer)
- https://docs.velopack.io/packaging/signing
- https://v2.tauri.app/distribute/sign/windows/
- https://docs.rs/ssign (community SimplySign tool)

**Microsoft Store**
- https://blogs.windows.com/windowsdeveloper/2025/09/10/free-developer-registration-for-individual-developers-on-microsoft-store/ (2025-09-10)
- https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account (ms.date 2025-09-17, updated 2026-07-17)
- https://learn.microsoft.com/en-us/windows/apps/publish/store-policies (v7.20, ms.date 2026-09-14)
- https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements (updated 2026-08-24)
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations (read 2026-10-21)
- https://learn.microsoft.com/en-us/uwp/api/Windows.ApplicationModel.StartupTask
- https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes (ms.date 2026-09-30)
- https://learn.microsoft.com/en-us/windows/uwp/packaging/self-install-package-updates
- https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli (ms.date 2026-10-08)
- https://blogs.windows.com/windowsdeveloper/2026/01/22/announcing-winapp-the-windows-app-development-cli/ (2026-01-22)
- https://blogs.windows.com/windowsdeveloper/2026/02/11/enhanced-developer-tools-on-the-microsoft-store/ (2026-02-11)
- https://v2.tauri.app/distribute/microsoft-store/
- https://learn.microsoft.com/en-ie/partner-center/health-report
- https://www.advancedinstaller.com/microsoft-store-private-app-distribution.html (private audience, secondary)
- https://www.techzine.eu/news/devices/63242/many-apps-will-now-bypass-microsofts-new-store-for-updates/ (2021)

**Installers and updaters**
- https://github.com/velopack/velopack/releases (1.2.161 on 2026-09-29; 1.3.39 prerelease on 2026-10-17)
- https://docs.velopack.io/packaging/deltas
- https://docs.velopack.io/packaging/installer
- https://v2.tauri.app/plugin/updater/
- https://v2.tauri.app/distribute/windows-installer/
- https://crates.io/crates/tauri (2.12.4)
- https://crates.io/crates/tauri-plugin-updater (2.14.0)
- https://github.com/tauri-apps/plugins-workspace/issues/2672 (AppImage-only delta issue)
- https://github.com/electron-userland/electron-builder (website/docs/features/auto-update.md, website/docs/nsis.md, nsisOptions.ts; releases 26.18.0 / 6.8.11 on 2026-10-20)
- https://learn.microsoft.com/en-us/windows/msix/overview
- https://jrsoftware.org/isdl.php (7.1.0, 2026-08-12)
- https://jrsoftware.org/isorder.php
- https://nsis.sourceforge.io/Download (3.13, 2026-09-27)
- https://docs.firegiant.com/wix/osmf/
- https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases

**Crash reporting and telemetry**
- https://sentry.io/pricing/ (read 2026-10-21)
- https://github.com/getsentry/sentry-native (0.17.4, 2026-10-16)
- https://docs.sentry.io/platforms/native/
- https://docs.sentry.io/platforms/native/data-management/store-minidumps-as-attachments/
- https://sentry.io/for/good
- https://blog.sentry.io/sentrys-eu-data-region-now-in-early-access
- https://crates.io/crates/sentry (0.49.3)
- https://crates.io/crates/sentry-rust-minidump (0.18.1)
- https://github.com/EmbarkStudios/crash-handling
- https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps
- https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-registerapplicationrestart
- https://aptabase.com/pricing (read 2026-10-21)
- https://www.edpb.europa.eu/documents/guideline/guidelines-22023-on-technical-scope-of-art-53-of-eprivacy-directive_en (v2.0, 2024-10-07)
- https://edpb.europa.eu/system/files/2024-10/edpb_guidelines_202302_technical_scope_art_53_eprivacydirective_v2_en_0.pdf
- https://gdpr-info.eu/art-13-gdpr/
- https://eur-lex.europa.eu/eli/dir/2019/770/oj (Digital Content Directive)

**Legal**
- https://www.xbox.com/en-US/developers/rules (Jan 2015)
- https://www.riotgames.com/en/legal (Aug 2018)
- https://store.steampowered.com/video_policy
- https://www.epicgames.com/site/en-US/fan-art-policy (403; text via search)
- https://www.epicgames.com/help/c-202300000001637/c-202300000001726/how-to-turn-off-licensed-audio-in-fortnite-a202300000010365
- https://help.ea.com/en/articles/security-and-rules/ea-content-policy/
- https://www.nintendo.co.jp/networkservice_guideline/en/index.html (2024-09-02)
- https://www.copyright.gov/dmca-directory/faq.html
- https://www.tubefilter.com/2020/11/11/twitch-dmca-takedown-tools/ (2020-11-11)
- https://edition.cnn.com/2020/08/14/tech/twitch-record-dmca-copyright-notices (2020-08-14)
- https://dsa.kemitchell.com/contents (DSA article index)
- https://eur-lex.europa.eu/eli/reg/2022/2065/oj
- https://ofcom.org.uk/online-safety/illegal-and-harmful-content/enforcing-the-online-safety-act-scrutinising-illegal-harms-risk-assessments
- https://www.scl.org/10433-buivids-a-further-over-extension-of-the-domestic-purposes-exemption/
- https://scheja-partners.de/en/insights/trends-takeaways/recording-phone-conversations-the-social-media-trend-with-legal-risks/
- https://hudsoncook.com/article/online-chat-features-and-californias-two-party-consent-rule-for-recording-conversations/
- https://www.federalregister.gov/documents/2025/04/22/2025-05904/childrens-online-privacy-protection-rule
- https://sota.io/blog/gdpr-art-8-childrens-consent-age-verification-parental-authorization-digital-services-developer-guide-2026
- https://www.enisa.europa.eu/news/the-cra-single-reporting-platform-is-launched (2026-09-11)
- https://www.dlapiper.com/en/insights/publications/law-in-tech/2026/cyber-resilience-act
- https://www.osborneclarke.com/insights/european-commission-publishes-new-guidelines-cyber-resilience-act (2026-09-23)
- https://www.european-cyber-resilience-act.com/Cyber_Resilience_Act_Preamble_11_to_20.html
- https://www.ibanet.org/European-Product-Liability-Directive-liability-for-software
- https://www.helpnetsecurity.com/2026/06/26/microsoft-windows-10-free-security-updates-esu-program/ (2026-06-26)

**Licensing**
- https://ffmpeg.org/legal.html
- https://github.com/obsproject/obs-studio (GPL-2.0; libobs headers GPLv2+; 32.2.2 on 2026-08-14)
- https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/LICENSE.txt
- https://www.via-la.com/licensing-2/avc-h-264/
- https://www.via-la.com/wp-content/uploads/avcweb.pdf
- https://fsl.software/
- https://www.gnu.org/licenses/gpl-faq.html#MereAggregation
