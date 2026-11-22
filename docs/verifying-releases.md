# Verify a release

The [release workflow](../.github/workflows/release.yml) builds every release on GitHub Actions
from its tag and attests where it came from. Check that before you run it.

## Check the checksum

In the directory with the download and the release's `SHA256SUMS`:

```sh
sha256sum --check --ignore-missing SHA256SUMS
```

On macOS, use `shasum -a 256 --check --ignore-missing SHA256SUMS`. On Windows, run
`(Get-FileHash <archive>).Hash` in PowerShell and compare it with the archive's line in
`SHA256SUMS`; the case doesn't matter.

## Check the attestation

Download the release's `attestation.json` too. Then, with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify <archive> --bundle attestation.json \
  --repo 265866/teamdiff \
  --signer-workflow 265866/teamdiff/.github/workflows/release.yml \
  --source-ref refs/heads/main
```

It needs no GitHub sign-in. It fails unless this repository's `release.yml`, running on `main`,
attested that exact file.
