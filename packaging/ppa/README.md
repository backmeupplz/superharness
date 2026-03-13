# Ubuntu PPA Publishing

This directory contains tooling for uploading superharness to a Launchpad PPA
so Ubuntu/Debian users can install it via `apt`.

## Quick start

```bash
# From the repo root:
packaging/ppa/build-ppa.sh 0.2.0 noble

# Then upload the resulting .changes file:
dput ppa:<your-launchpad-id>/superharness \
    /tmp/ppa-build-superharness-0.2.0/superharness_0.2.0-1_source.changes
```

## Prerequisites

```bash
sudo apt-get install devscripts debhelper dput gpg curl
```

## Step-by-step setup

### 1. Create a Launchpad account

Go to <https://launchpad.net> and register. Note your Launchpad ID (`<lp-id>`).

### 2. Activate a PPA

Visit <https://launchpad.net/~<lp-id>/+activate-ppa> and create a PPA.
Suggested name: `superharness`.

The install URL for users will be:
```
ppa:<lp-id>/superharness
```

### 3. Generate and upload a GPG key

```bash
# Generate if you don't have one
gpg --full-gen-key

# Get your key ID (long format)
gpg --list-secret-keys --keyid-format LONG

# Upload to Ubuntu's keyserver
gpg --keyserver keyserver.ubuntu.com --send-keys <YOUR_KEY_ID>
```

Then add it to Launchpad:
- Go to <https://launchpad.net/~<lp-id>/+editpgpkeys>
- Paste your key fingerprint and confirm via the email Launchpad sends.

### 4. Configure dput

Create or extend `~/.dput.cf`:

```ini
[superharness-ppa]
fqdn            = ppa.launchpad.net
method          = ftp
incoming        = ~<lp-id>/ubuntu/superharness
login           = anonymous
allow_unsigned_uploads = 0
```

### 5. Build the source package

```bash
# Target Ubuntu Noble (24.04 LTS)
packaging/ppa/build-ppa.sh 0.2.0 noble
```

For multiple Ubuntu releases, run the script once per series:
```bash
for series in focal jammy noble; do
    packaging/ppa/build-ppa.sh 0.2.0 "$series"
done
```

### 6. Upload to Launchpad

```bash
dput superharness-ppa \
    /tmp/ppa-build-superharness-0.2.0/superharness_0.2.0-1_source.changes
```

Launchpad will email you once the package is accepted and built.

### 7. Users install via apt

```bash
sudo add-apt-repository ppa:<lp-id>/superharness
sudo apt-get update
sudo apt-get install superharness
```

## Directory layout

```
packaging/
  debian/
    debian/
      control      — package metadata
      rules        — build instructions (cargo build --release)
      changelog    — Debian changelog (update on each release)
      compat       — debhelper compat level (13)
      copyright    — upstream license info
  ppa/
    build-ppa.sh   — script that downloads tarball, injects debian/, runs debuild
    README.md      — this file
```

## Bumping the version

When releasing a new version:

1. Update `debian/changelog` (use `dch -v <new-version>-1` or edit manually).
2. Run `packaging/ppa/build-ppa.sh <new-version> noble`.
3. Upload the resulting `.changes` file with `dput`.

## Notes

- The `build-ppa.sh` script builds a **source** package only (no binaries uploaded).
  Launchpad compiles it for each supported architecture.
- Rust must be available during the Launchpad build. The `control` file lists
  `cargo` and `rustc` as build dependencies; Launchpad's Noble builders have
  them available via the Ubuntu archive.
- If the Launchpad Rust version is too old for edition 2024, pin a newer rustc
  via the `rustup` bootstrap or use the `rust-team/rust` PPA as a build dependency.
