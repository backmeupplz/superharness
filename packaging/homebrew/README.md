# Homebrew Packaging

superharness is distributed via a Homebrew tap — a personal formula repository hosted on GitHub.

## Using the tap

```bash
brew tap backmeupplz/superharness
brew install superharness
```

## Setting up the tap repository

1. Create a new GitHub repository named **`homebrew-superharness`** under your GitHub account
   (the `homebrew-` prefix is required by Homebrew convention).

2. Inside that repo create the path `Formula/superharness.rb` and paste the contents of
   `packaging/homebrew/superharness.rb` from this repository as a starting point.

3. Update the SHA256 placeholders with real values (see "Computing SHA256" below).

4. Commit and push to the `main` branch.

5. Users can now install with:
   ```bash
   brew tap backmeupplz/superharness
   brew install superharness
   ```

## Computing SHA256 values

After each release, download all four binaries and run:

```bash
VERSION=0.2.0
BASE="https://github.com/backmeupplz/superharness/releases/download/v${VERSION}"

for target in x86_64-apple-darwin aarch64-apple-darwin \
              x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    curl -fSL -o "superharness-v${VERSION}-${target}" \
        "${BASE}/superharness-v${VERSION}-${target}"
    sha256sum "superharness-v${VERSION}-${target}" 2>/dev/null \
        || shasum -a 256 "superharness-v${VERSION}-${target}"
done
```

Paste the resulting hashes into the `sha256` lines in `superharness.rb`.

## Automated updates via GitHub Actions

The `homebrew.yml` workflow in this repo automatically regenerates the formula and pushes it
to `backmeupplz/homebrew-superharness` whenever a new release is published. It requires a
`HOMEBREW_TAP_TOKEN` secret (a GitHub personal access token with `repo` scope on the tap repo).

## Local development / testing

```bash
# Audit the formula (requires Homebrew installed)
brew audit --new-formula packaging/homebrew/superharness.rb

# Install from local file (after updating SHAs)
brew install --build-from-source packaging/homebrew/superharness.rb
```
