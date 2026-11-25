# Kinana 0.3.0 beta

This beta adds an optional import flow for project-local skills, batch approval of skill descriptions, and more informative audit findings. The download is Developer ID signed and notarized for Apple Silicon. It requires macOS 27.0 or later and was verified on macOS 27.2.1.

## What’s new

- **Import skills when adding a project.** When Kinana finds project-local skills, **Review and add** opens the same scan review used for other imports. Audit the files, review duplicates, and choose which version to bring into the vault. Importing leaves the project untouched. An exact content match links the project-local entry to its vault skill.
- **Classify only what’s pending.** Start from the vault or Settings. **Classify all skills** reruns classification across the whole vault. The confirmation shows the estimated input, model, and effort where applicable. Settings shows the saved Claude choice, with a save action when you change it.
- **Keep going after a per-skill failure.** The classification window reports elapsed time for the current skill and supports cancellation while a call is running. After retries, it skips a skill with an invalid reply or timeout and gives the reason. An explicit Claude refusal has its own reason. You can choose keyword classification for a single skipped skill. Provider-wide errors stop the batch and show the reason.
- **Approve descriptions individually, in a batch, or automatically.** By default, eligible Claude or Pi proposals for clean skills and skills that need review are approved together in one undoable Git commit. High-risk proposals wait for review. You can turn off automatic approval in Settings. An unchanged proposal leaves an existing approved description in place.
- **Read an audit finding in context.** Skill details show the rule, file, line, and a short excerpt. Repeated findings are grouped by rule and file. Secret-related findings show variable names and matching forms while hiding literal values. Refined rules distinguish common non-secret reads from secret-like names. Unpinned package execution is marked for review.
- **Try Pi for classification.** Pi is experimental and off by default. Kinana can list providers and models from the reviewed Pi 1.5.2 build without sending skill text. Codex remains available for activation and the router. Its direct classification option is disabled because Kinana cannot verify that it disallows local file reads.

## Audit rules and CLI contract

The audit again marks secret-like names such as `apiKey`, `githubToken`, and `GITHUBPAT` as high risk. Existing audit results keep their stored levels until you run **Settings → Maintenance → Refresh audit**. The preview shows which results would change.

The CLI uses JSON contract 11. The classification result includes `auto_approved`, the Pi confirmation includes `model_default`, and secret-related audit findings include `forms`.

## Known issues

- **High-risk activation.** There is no extra confirmation for activating a high-risk skill. Read the audit reasons before activation.
- **Audit false positives.** Ordinary environment reads may be flagged as high risk when a variable name resembles a secret. Inspect the matching name and form in the finding.
- **Pi compatibility.** The reviewed version is 1.5.2. Pi 1.6.0 is under review. If provider or model discovery is unavailable, Settings explains why and displays manual fields. **Pi default** uses the selected provider’s built-in default in 1.5.2. It may differ from a model you selected in Pi’s own settings.
- **Classification.** Prompt template 1 ships in this beta. Template 2, revised description guidance, and a proposed description length target await evaluation. Claude effort selection requires a Claude Code version that lists the option in its help. Skipped reasons are visible in the results window until it closes.
- **Import and review.** Selecting an agent immediately after a scan and editing a proposed description before approval are deferred.
- **Browsing and projects.** A category browser, explanations for suggested skill pairs, a skill-coverage dashboard, and clearer wording for **Not activated yet** are deferred.

## Download and install

| File | Purpose |
| --- | --- |
| `Kinana-0.3.0-beta-arm64.dmg` | Signed and notarized app for Apple Silicon |
| `SHA256.txt` | SHA-256 checksums for the disk image and notices |
| `THIRD_PARTY_NOTICES.md` | Licenses and notices for bundled dependencies |

Download all three files from the [release page](https://github.com/khaled1174/kinana/releases/tag/v0.3.0-beta) and run `shasum -a 256 -c SHA256.txt` in their folder. Open the disk image, drag **Kinana.app** to **Applications**, and launch it from there. The first launch may ask you to confirm opening an app downloaded from the internet.

See the [README](https://github.com/khaled1174/kinana/blob/main/README.md) for screenshots, setup, and the privacy and security model.
