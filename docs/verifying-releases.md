# Verify a release

The [release workflow](../.github/workflows/release.yml) builds every release on GitHub Actions
from its tag and attests where it came from. Check that before you run it.

## Check the checksum

In the directory with the download and the release's `SHA256SUMS.txt`:

```sh
sha256sum --check --ignore-missing SHA256SUMS.txt
```

On macOS, use `shasum -a 256 --check --ignore-missing SHA256SUMS.txt`. On Windows, run
`(Get-FileHash <file>).Hash` in PowerShell and compare it with the file's line in
`SHA256SUMS.txt`; the case doesn't matter.

## Check the attestation

Download the release's `attestation.json` too. Then, with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify <file> --bundle attestation.json \
  --repo 265866/teamdiff \
  --signer-workflow 265866/teamdiff/.github/workflows/release.yml \
  --source-ref refs/heads/main
```

It needs no GitHub sign-in. It fails unless this repository's `release.yml`, running on `main`,
attested that exact file.
