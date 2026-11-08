# Release runbook

This is the release runbook for nca-rs. The tag-driven
[`Release` workflow](../.github/workflows/release.yml) builds `nca` and
`nca-gui` for Linux x86_64, Windows x86_64, and macOS, writes
`SHA256SUMS.txt`, and publishes a **normal, non-draft, non-prerelease** GitHub
Release marked Latest.

The workflow publishes automatically when a matching tag is pushed. It does
not code-sign, notarize, or create an installer, and it has no manual approval
step. Do not push the release tag until every gate below passes.

nca-rs is a read-only analyzer. There is no controller connection, so there is
no hardware-validation gate. The gate that replaces it is an
**advisory/accuracy gate**: reports are advisory, coverage is not a safety
verdict, and no build may present a green "SAFE" state.

## 1. Version and tag

- [ ] Decide the version and update `[workspace.package] version` in
      `Cargo.toml`. It is the single version for every workspace crate.
- [ ] Refresh the lock file with a bounded check, for example
      `cargo check --offline -p nc-cli`, then commit `Cargo.toml` and
      `Cargo.lock` in the release commit.
- [ ] Confirm the tag is the workspace version with a leading `v`
      (`0.5.0` -> `v0.5.0`). The workflow accepts
      `vMAJOR.MINOR[.PATCH][-pre]` and rejects anything else.
- [ ] Create an **annotated** tag on the exact commit that passed CI:

      git tag -a v0.5.0 -m "nca-rs v0.5.0"

- [ ] Sign the tag when a signing key is configured:

      git tag -s v0.5.0 -m "nca-rs v0.5.0"
      git verify-tag v0.5.0

      Signing is optional for the workflow but should be recorded. A
      lightweight tag is rejected by the prepare job, so always use `-a` or
      `-s`.
- [ ] Never move, force-push, or reuse a published tag. If the tag is wrong and
      nothing has been published or downloaded, delete it and start over.

## 2. Changelog and release notes

- [ ] Update `CHANGELOG.md`: move the relevant `Unreleased` entries under the
      new version and date. Keep the Added, Changed, Fixed, Safety, and Known
      limitations groups.
- [ ] Verify the known limitations match the code and docs. In particular,
      repeat that camera and stock-mesh frame targets are unmeasured, mid-run
      cancellation is absent (D034), the SIMULATE session build is synchronous
      (D035), the 1M-line memory target misses (D030), `box54` model findings
      are noisy, and dedicated fixtures are still missing (D033).
- [ ] Do not claim hardware validation or a safety guarantee anywhere in the
      notes. nca-rs has no controller path to validate.
- [ ] The workflow asks GitHub to generate release notes from commits and
      merged pull requests. The curated `body` in the workflow is prepended to
      those generated notes. Review the published body after the workflow
      completes and edit the release notes if the generated section is noisy.

## 3. Local gates

Run from the repository root with the pinned toolchain from
`rust-toolchain.toml`:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p nc-cli -p nc-app
cargo doc --workspace --no-deps
```

- [ ] Every command passes from a clean checkout of the release commit.
- [ ] Optional: `cargo deny check` when `cargo-deny` is installed. `deny.toml`
      covers licenses, advisories, and duplicate dependencies.
- [ ] Do not tag while any gate is failing, including a flaky test. Re-run it
      and record the result.
- [ ] Confirm the release profile is still `lto = "thin"` with
      `codegen-units = 1`; if it changed, re-run the performance checks in
      `docs/PERFORMANCE.md`.

## 4. CI green on the exact commit

- [ ] In the Actions tab, open the `CI` run for the release commit.
- [ ] `Format`, `Clippy (Linux)`, `Test (Linux)`, `GUI build (Linux)`, and
      `Build (ubuntu-latest)`, `Build (windows-latest)`, and
      `Build (macos-latest)` must all pass.
- [ ] Confirm the run is for the same commit SHA that will be tagged. A green
      run on a different SHA does not count.
- [ ] Wait for in-flight runs to finish before tagging.
- [ ] CI proves the debug build on three platforms; it does not exercise the
      release archives. The local release build in section 3 is required.

## 5. Fixtures, goldens, and screenshots

- [ ] Run the golden suite and the real-file regressions:

      cargo test -p nc-fixtures --test golden
      cargo test -p nc-fixtures --test real_file

- [ ] Review any changed `expected.json`. A golden is never auto-blessed: the
      report must be checked against the source, the rule definitions, and the
      intended behavior change. The write helper is:

      NCA_WRITE_GOLDENS=1 cargo test -p nc-fixtures write_goldens_for_review -- --ignored

      Run it only after the report has been reviewed. The normal suite never
      writes goldens.
- [ ] `box54-lid-50cm` and `makera-latchbase` must stay deterministic. They do
      not have blessed goldens; `box54` is a determinism, cap, and performance
      corpus, and `LatchBase.nc` is the Makera dialect fixture.
- [ ] Review the captures in `docs/images/` against
      [`screenshots.md`](screenshots.md). The reference OS is Linux. Do not
      commit a capture that contains a private path, file content, or desktop
      chrome.
- [ ] Confirm every rule that cannot run is shown as a skip with a reason. A
      coverage view with a percentage, progress bar, or green "SAFE" state is a
      release blocker.

## 6. Advisory and accuracy gate

This gate replaces the controller-hardware gate used by projects that talk to
a machine. nca-rs has no transport, no motion, and no G-code writing, so the
release question is whether the analyzer is honest and accurate enough to
publish, not whether a controller accepted it.

- [ ] Read the release report for a representative fixture and confirm every
      finding has a stable rule id, severity, basis, source line, and evidence.
- [ ] Confirm missing tool, stock, machine, or material data produces a named
      skip, not an invented value.
- [ ] Confirm the app still presents no green "SAFE" state or coverage
      percentage. Coverage counts and skip reasons are the only summary.
- [ ] Confirm the product still has no serial, controller, streaming, jog, DRO,
      network, or G-code-writing path. Search the dependency tree and the
      binaries before release; a new transport dependency is a blocker.
- [ ] Review the known limitations in `CHANGELOG.md` and
      `docs/PERFORMANCE.md`. Do not describe a report as a safety verdict, a
      validation, or a guarantee.
- [ ] Re-read the `box54` caveat: its model-rule findings are noisy and are not
      blessed. Do not use its clean or noisy result as a marketing claim.
- [ ] If the release changes rule behavior, record which goldens changed and
      why in the release record.

## 7. Run the release workflow

- [ ] Push the annotated tag:

      git push origin v0.5.0

      The workflow publishes automatically. There is no draft to approve and no
      separate publish button.
- [ ] Alternatively, open Actions -> `Release` -> `Run workflow` from the
      repository and enter the existing tag in the `tag` input. The workflow
      does not create tags.
- [ ] Watch the jobs: `Prepare release`, `Package (Linux x86_64)`,
      `Package (Windows x86_64)`, `Package (macOS)`, and
      `Publish GitHub Release`.
- [ ] The repository must allow the workflow token `contents: write` for the
      release job. If Actions permissions are restricted to read-only, the
      release job fails with a 403 and no release is created.
- [ ] The prepare job enforces the tag pattern, requires an annotated tag
      object, checks that the checked-out commit is the tag commit, and checks
      that `v<Cargo.toml version>` equals the tag. It fails on a lightweight
      tag or a version mismatch.
- [ ] The Linux job installs the same apt dependencies as CI, runs
      `cargo build --release -p nc-cli -p nc-app`, and stages `nca`, `nca-gui`,
      `README.md`, both licenses, `NOTICE`, and `PACKAGING.md` in
      `nca-rs-<tag>-linux-x86_64/`.
- [ ] The Windows job builds both binaries and produces a zip with the same
      documentation. The macOS job builds both binaries and produces a tar.gz;
      its asset name records the runner architecture.
- [ ] The final job downloads all artifacts, verifies exactly one archive per
      platform, writes `SHA256SUMS.txt`, and publishes the release. A failed
      package job means the release job never runs.
- [ ] The publish step sets `overwrite_files: false`. A rerun against a release
      that already has assets fails instead of replacing them; cut a new patch
      release instead.

Do not test on the release tag. The workflow publishes every matching tag as a
normal release, including a `-pre` tag, so a scratch tag is not a safe staging
area in this repository. If the workflow itself must be exercised, use a
scratch repository or fork and a throwaway tag such as `v0.0.0-scratch.1`;
delete that release and tag immediately. Never move or reuse a published tag.

## 8. Artifact inspection

Expected assets for `v0.5.0`:

| Platform | Asset | Contents |
|---|---|---|
| Linux x86_64 | `nca-rs-v0.5.0-linux-x86_64.tar.gz` | `nca`, `nca-gui`, README, licenses, NOTICE, `PACKAGING.md` |
| Windows x86_64 | `nca-rs-v0.5.0-windows-x86_64.zip` | `nca.exe`, `nca-gui.exe`, README, licenses, NOTICE, `PACKAGING.md` |
| macOS arm64 | `nca-rs-v0.5.0-macos-aarch64.tar.gz` | `nca`, `nca-gui`, README, licenses, NOTICE, `PACKAGING.md` |
| Checksums | `SHA256SUMS.txt` | SHA-256 for the three archives |

`macos-latest` is an Apple Silicon runner at the time of writing, so the
expected architecture is `aarch64`. If the runner image changes to Intel, the
asset becomes `nca-rs-v0.5.0-macos-x86_64.tar.gz`; the workflow normalizes
`uname -m` to `x86_64` or `aarch64`. Do not rename an asset after publication.

- [ ] Download every asset from the published release, not from a local build.
- [ ] List each archive and confirm it unpacks to exactly one top-level
      directory named after the archive without its extension.
- [ ] Confirm the archive contains `nca`, `nca-gui`, `README.md`,
      `LICENSE-APACHE`, `LICENSE-MIT`, `NOTICE`, and `PACKAGING.md`. There is no
      `target/` tree, no `.pdb`, no test binary, and no local config.
- [ ] Keep `nca` and `nca-gui` side by side. `nca view` starts the sibling
      `nca-gui` binary.
- [ ] Linux: `file nca nca-gui` reports x86-64 ELF executables. Run
      `./nca --version` and `./nca rules`. Start `./nca-gui` on a host with a
      Wayland or X11 session and a Vulkan driver. The GUI uses the host GPUI
      runtime libraries; it does not bundle GTK, Wayland, Vulkan, or Mesa.
- [ ] Linux CLI: run `nca check` twice on the same fixture and compare the JSON
      bytes. Reports must be deterministic and must not contain an absolute
      path, timestamp, hostname, or locale-formatted number.
- [ ] Windows: confirm the zip entries use forward slashes and the executable
      starts on a clean VM. `Get-AuthenticodeSignature` must report
      `NotSigned`; that is expected, not a defect.
- [ ] macOS: confirm the tar contains both binaries and no `.app` bundle.
      `codesign -dv` reports an unsigned binary and `spctl -a -vv` rejects it;
      that is expected. The first launch needs a per-file quarantine removal
      after checksum verification (see `packaging/README.md`), never a
      system-wide Gatekeeper change.
- [ ] Confirm `nca --version` reports `0.5.0` and `nca rules` lists the full
      45-rule catalog. A version or rule-count mismatch means the wrong ref was
      built.

## 9. Checksums

- [ ] `SHA256SUMS.txt` is attached to the release and covers exactly the three
      archives.
- [ ] Download the archives and `SHA256SUMS.txt` into one directory:

      sha256sum -c SHA256SUMS.txt

- [ ] On Windows, compare `Get-FileHash -Algorithm SHA256` for each archive with
      the matching line.
- [ ] Record the three checksums in the release record. If a checksum does not
      match, do not announce the release; follow section 12.
- [ ] After publication, never replace an asset under the same tag. Cut a new
      patch release instead.

## 10. Unsigned-build caveat

The workflow uses only `GITHUB_TOKEN` and performs no signing. State this in the
release notes and in every announcement:

- Linux archives have no signature. The binaries use the host Wayland/X11 and
  Vulkan libraries; there is no `.deb`, AppImage, Flatpak, or Snap package.
- Windows executables have no Authenticode signature. SmartScreen warns on
  first launch.
- macOS binaries are not code-signed or notarized. Gatekeeper blocks the first
  launch until the user allows those files.
- There is no installer and no auto-update channel.

Never tell users to disable Gatekeeper or SmartScreen system-wide; only
per-file or per-app overrides are acceptable. Users should verify
`SHA256SUMS.txt` before running anything. Code signing and installers are
follow-ups that need maintainer certificates and managed secrets, deliberately
out of scope for this workflow.

## 11. Verify the published release

- [ ] The release is not a draft, is not marked pre-release, and is marked
      Latest.
- [ ] The tag is the annotated release tag and points at the release commit.
- [ ] All four assets are present: three platform archives and
      `SHA256SUMS.txt`.
- [ ] The body states that the builds are unsigned, that the Linux GUI needs
      host Wayland/X11 and Vulkan libraries, that there is no installer, that
      the analyzer is advisory and not a safety verdict, and that downloads
      must be verified against `SHA256SUMS.txt`.
- [ ] The generated notes do not contain an incorrect hardware-validation or
      safety claim. Edit the release notes if they do; do not replace assets.
- [ ] Announce the exact tag and the three checksums.

## 12. Rollback

Before the tag is pushed:

- [ ] Delete the local tag. Nothing has been published.

After the tag is pushed but before the release job finishes:

- [ ] Cancel the run. If no release was created, fix the cause and, if nothing
      was downloaded from the tag, delete and recreate the tag on the corrected
      commit. This is the only time moving a tag is acceptable.
- [ ] Do not publish a partial release. The release job requires all three
      package jobs, so a failed package job leaves no release.

After the release is published:

- [ ] Do not silently replace binaries under an existing tag. Users and mirrors
      may already hold copies, and the published checksums would no longer
      match.
- [ ] For a broken but not dangerous build: add a warning at the top of the
      release notes, mark the release as a pre-release, and publish a fixed
      patch release from a revert or fix commit.
- [ ] For a dangerous or misleading build: delete the release assets or the
      release, keep the tag for traceability, add a prominent warning, and
      notify users through the same channels used for the announcement.
- [ ] `git revert` the bad change on `main`; never force-push `main` and never
      re-point a published tag.
- [ ] Record the rollback and the replacement tag in the release record.

## 13. Release record

Copy this table into the release issue, the release notes, or the maintenance
log.

| Field | Value |
|---|---|
| Tag | `v0.5.0` |
| Commit SHA | |
| CI run URL | |
| Release run URL | |
| `nca-rs-v0.5.0-linux-x86_64.tar.gz` SHA-256 | |
| `nca-rs-v0.5.0-windows-x86_64.zip` SHA-256 | |
| `nca-rs-v0.5.0-macos-aarch64.tar.gz` SHA-256 | |
| Golden fixtures | pass / fail |
| `box54` and `LatchBase` regressions | pass / fail |
| Screenshots reviewed | yes / no |
| Advisory gate | pass / fail |
| Signed / notarized | no |
| Hardware validated | not applicable (no controller path) |
| Published by | |
| Notes | |
