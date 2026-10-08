# From zero to restore in CI

> Phase 6 guide. Run it as is, top to bottom, in a bash terminal (Linux or macOS).
> Last updated: 2026-10-07

By the end you will have a running registry, a private feed, a CI service account with a token limited to its packages, a published package, and a project that restores it the way a CI agent would. No web UI: just `onepack` and `dotnet`.

Estimated time: 15 minutes (plus the first build).

## 0. Requirements

- [rustup](https://rustup.rs) (the repository pins the Rust version).
- .NET SDK 8 or later (`dotnet --version`).
- `git` and `curl`.

## 1. Build the binaries

```bash
git clone https://github.com/Khr0x/onepack.git
cd onepack
cargo build --release -p onepack-server -p onepack-cli
export PATH="$PWD/target/release:$PATH"
onepack --version
onepackd --version
```

`onepackd` is the server; `onepack` is the CLI. A workstation only needs `onepack`.

## 2. Start the server

```bash
export DEMO="$HOME/onepack-demo"
mkdir -p "$DEMO"
onepackd init --data-dir "$DEMO/data"
```

`init` creates the data directory and writes the initial administrative credential to `$DEMO/data/initial-admin-token`. In **another terminal**, start the server and leave it running:

```bash
export DEMO="$HOME/onepack-demo"
export PATH="$PWD/target/release:$PATH"   # from the repository directory
onepackd serve --data-dir "$DEMO/data" --public-url http://127.0.0.1:8080
```

> This guide uses HTTP on `127.0.0.1` to try things locally. On a real server, put `onepackd` behind TLS and use as `--public-url` the `https://` URL clients will reach it at.

Go back to the first terminal.

## 3. Set up the CLI as administrator

```bash
onepack context add local --url http://127.0.0.1:8080
onepack login --token-stdin < "$DEMO/data/initial-admin-token"
```

`login` validates the token and stores it in the system keychain. If it answers `KEYCHAIN_UNAVAILABLE` (e.g. on a server without a graphical session), pass the token in an environment variable instead:

```bash
export ONEPACK_TOKEN="$(cat "$DEMO/data/initial-admin-token")"
```

Check the identity and, once the credential is stored, delete the initial credential file:

```bash
onepack whoami
rm "$DEMO/data/initial-admin-token"   # only if you used `login`; with ONEPACK_TOKEN, store it in a secrets manager first
```

`whoami` should show `admin (user, administrator)`.

## 4. Create the feed and the CI account

```bash
onepack feed create payments
onepack principal create ci-payments --kind service
onepack grant add --principal ci-payments --feed payments --role publisher --publish-pattern 'Hemia.Payments.*'
CI_TOKEN="$(onepack token create --principal ci-payments --name github-actions --expires-in-days 90)"
export CI_TOKEN
```

The token is shown only once; in a real setup it would go straight into the pipeline secret. With `--publish-pattern`, the CI account can only publish `Hemia.Payments.*` ids to `payments`.

Check what it can do:

```bash
onepack --token-env CI_TOKEN doctor --feed payments --require publish
```

Every check should report `ok`, except `nuget-config`, which warns (`WARN`) because there is no `NuGet.Config` yet; the final line is `No failures.` The token never appears in the output: only its id (`opk_<id>_…`).

## 5. Publish a package the way the pipeline would

```bash
mkdir -p "$DEMO/src"
dotnet new classlib -n Hemia.Payments.Core -o "$DEMO/src/Hemia.Payments.Core"
dotnet pack "$DEMO/src/Hemia.Payments.Core" -c Release -o "$DEMO/out" -p:Version=1.0.0
onepack --token-env CI_TOKEN package push --feed payments "$DEMO"/out/*.nupkg --skip-existing-identical
onepack package list --feed payments
```

`--skip-existing-identical` means retrying the pipeline with the same package does not fail. With an already published version and different content it fails (exit 6): versions are immutable.

## 6. Prepare the consumer project

```bash
dotnet new console -n Consumer -o "$DEMO/consumer"
cd "$DEMO/consumer"
onepack nuget init --feed payments --pattern 'Hemia.Payments.*' --dry-run
onepack nuget init --feed payments --pattern 'Hemia.Payments.*' --yes
cat NuGet.Config
dotnet add package Hemia.Payments.Core --version 1.0.0 --no-restore
```

`--dry-run` shows the diff without writing. The resulting `NuGet.Config` declares the `onepack_payments` source and a `packageSourceMapping`: `Hemia.Payments.*` ids are only resolved from onepack and everything else still comes from nuget.org. **It contains no secrets**, so it can be committed to the repository.

## 7. Restore locally

```bash
onepack exec --feed payments -- dotnet restore
dotnet build --no-restore
```

`onepack exec` sets the credential only in the environment of `dotnet restore`. Afterwards it is neither in the shell nor on disk:

```bash
env | grep NuGetPackageSourceCredentials || echo "no credentials in the shell"
grep -c Password NuGet.Config || true
```

## 8. Restore like a CI agent

CI does not use `onepack`: NuGet reads the credential from the `NuGetPackageSourceCredentials_<source>` variable, which the pipeline fills from a secret. Simulate it with an empty cache and only the CI token:

```bash
cd "$DEMO/consumer"
rm -rf obj bin
env -u ONEPACK_TOKEN \
  NUGET_PACKAGES="$(mktemp -d)" \
  NUGET_HTTP_CACHE_PATH="$(mktemp -d)" \
  NuGetPackageSourceCredentials_onepack_payments="Username=ci;Password=$CI_TOKEN" \
  dotnet restore
dotnet build --no-restore
```

The restore downloads `Hemia.Payments.Core 1.0.0` from onepack with the CI credential.

In GitHub Actions, with the server reachable over HTTPS and the token stored as the `ONEPACK_TOKEN` secret:

```yaml
jobs:
  build:
    runs-on: ubuntu-24.04
    env:
      NuGetPackageSourceCredentials_onepack_payments: "Username=ci;Password=${{ secrets.ONEPACK_TOKEN }}"
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-dotnet@v4
        with:
          dotnet-version: 8.0.x
      - run: dotnet restore
      - run: dotnet build --no-restore
```

To publish from the pipeline, either `dotnet nuget push out/*.nupkg --source onepack_payments --api-key "$ONEPACK_TOKEN"` or `onepack --url https://packages.example.com package push --feed payments out/*.nupkg --skip-existing-identical` works (`onepack` reads `ONEPACK_TOKEN`).

## 9. Check and audit

```bash
cd "$DEMO/consumer"
onepack --token-env CI_TOKEN doctor --feed payments --require publish
onepack audit list --limit 10
```

Now `doctor` reports no failures, including `nuget-config` and `source-mapping`. The audit log shows the creation of the feed, the principal, the token, the grant and the publish, each with its actor.

## 10. Revoke and clean up

```bash
onepack token list --principal ci-payments
onepack token revoke <ID> --yes            # the ID is the first column
onepack --token-env CI_TOKEN whoami; echo "exit: $?"   # 3: revoked credential
```

To finish, stop `onepackd` (Ctrl+C in its terminal) and delete the test directory:

```bash
onepack logout                      # only if you used `login` (with ONEPACK_TOKEN: unset ONEPACK_TOKEN)
onepack context remove local --yes
rm -rf "$DEMO"
```

## If something fails

- `onepack doctor --feed payments` tells you which check fails and what to do.
- Every CLI error includes a stable code, a suggested action and a `request_id` that also appears in the `onepackd` log. Exit codes are in [docs/cli.md](cli.md#exit-codes).
