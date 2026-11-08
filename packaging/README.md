# Release packaging

The `Release` workflow (`.github/workflows/release.yml`) builds `nca` and
`nca-gui` in release mode on Linux, Windows, and macOS, writes
`SHA256SUMS.txt`, and publishes the archives as a normal GitHub Release. The
builds are unsigned: there is no code signing, notarization, Authenticode, or
installer. Maintainer gates are in `docs/RELEASE.md` in the repository
(<https://github.com/LeandroMarceddu/nca-rs/blob/main/docs/RELEASE.md>).

## Artifacts

| Platform | Asset | Contents |
|---|---|---|
| Linux x86_64 | `nca-rs-<tag>-linux-x86_64.tar.gz` | `nca`, `nca-gui`, docs, notices |
| Windows x86_64 | `nca-rs-<tag>-windows-x86_64.zip` | `nca.exe`, `nca-gui.exe`, docs, notices |
| macOS | `nca-rs-<tag>-macos-<arch>.tar.gz` | `nca`, `nca-gui`, docs, notices |

`<tag>` includes the leading `v` (for example `v0.5.0`). `<arch>` is the
architecture reported by the macOS runner: `aarch64` on Apple Silicon, or
`x86_64` if an Intel runner is used. The release job also publishes
`SHA256SUMS.txt`, which is not inside any archive.

Every archive unpacks to one top-level directory named after the archive
without its extension. Keep `nca` and `nca-gui` side by side: `nca view` starts
the sibling `nca-gui` binary.

## Linux

```sh
tar -xzf nca-rs-v0.5.0-linux-x86_64.tar.gz
cd nca-rs-v0.5.0-linux-x86_64
mkdir -p ~/.local/bin
install -m 755 nca nca-gui ~/.local/bin/
```

`nca` is a standalone CLI. `nca-gui` uses the host GPUI runtime: a Wayland or
X11 session, a working Vulkan driver, and the distribution packages for
Wayland, XKB, X11/xcb, fontconfig, FreeType, and udev. On Debian/Ubuntu, the
usual runtime set is:

```sh
sudo apt install libwayland-client0 libxkbcommon0 libxkbcommon-x11-0 \
  libx11-6 libx11-xcb1 libxcb1 libxcb-render0 libxcb-shape0 \
  libxcb-xfixes0 libfontconfig1 libfreetype6 libudev1 libvulkan1
```

The exact set depends on the distribution and the GPU driver. The build does
not bundle GTK, Wayland, Vulkan, or Mesa.

## Windows

Extract the zip and run `nca.exe` or `nca-gui.exe`. The executables are
portable. They have no Authenticode signature, so SmartScreen warns on first
launch: choose "More info", verify the checksum, then "Run anyway". Do not
disable SmartScreen system-wide. There is no MSI or MSIX installer.

## macOS

```sh
tar -xzf nca-rs-v0.5.0-macos-aarch64.tar.gz
cd nca-rs-v0.5.0-macos-aarch64
```

The archive contains bare binaries, not a `.app` bundle; run them from a
terminal. The binaries are not code-signed or notarized. Gatekeeper blocks the
first launch of a downloaded binary. After verifying the checksum, remove the
quarantine attribute for those files only:

```sh
xattr -d com.apple.quarantine nca nca-gui
```

Do not disable Gatekeeper system-wide. A signed and notarized build is a
follow-up.

## Checksums

Download `SHA256SUMS.txt` from the same release and verify every archive
before running it:

```sh
sha256sum -c SHA256SUMS.txt
```

On Windows:

```powershell
Get-FileHash .\nca-rs-v0.5.0-windows-x86_64.zip -Algorithm SHA256
```

Compare the value with the matching line in `SHA256SUMS.txt`.

## Not included

- Code signing, notarization, and Windows Authenticode.
- Installers (MSI/MSIX, deb, rpm) and AppImage, Flatpak, or Snap packages.
- Auto-update metadata or an update channel.
- Controller connection, streaming, or any machine-control path. nca-rs is
  read-only by design.
