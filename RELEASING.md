# Releasing Clannon

This is the maintainer contract for the Linux local alpha. It deliberately does
not describe hosted deployment, package repositories, background services,
automatic updates, or platforms outside the supported release artifact.

## Release model

- Use semantic `0.y.z` versions and an annotated `v0.y.z` tag.
- Treat a minor bump as the compatibility boundary before 1.0. Reserve patch
  releases for compatible fixes unless preserving behavior would be unsafe.
- Support only the newest published alpha. Never move or reuse a published tag
  or replace its artifacts silently.
- Publish `clannon-v0.y.z-x86_64-unknown-linux-musl.tar.gz` and a companion
  `SHA256SUMS`. The archive has one same-named root directory containing
  `clannon`, `README.md`, `PROJECT.md`, `SECURITY.md`, `LICENSE-MIT`,
  `LICENSE-APACHE`, `BUILD-INFO.txt`, and `DEPENDENCIES.txt`. The dependency
  inventory is not an SBOM or third-party license bundle; do not market it as
  either.
- Keep GitHub release notes as the alpha changelog. State behavior changes,
  security fixes, migration or data-loss consequences, and known limitations.

## Human and repository preflight

Do not tag or publish while any item here is unresolved:

- The release version is intentional and matches the workspace package version.
- `LICENSE-MIT` and `LICENSE-APACHE` contain the approved license texts and the
  legal copyright attribution agrees with the manifest's
  `MIT OR Apache-2.0` declaration.
- A legal review has confirmed every third-party distribution obligation and
  added any required notices or license material. `DEPENDENCIES.txt` is an
  inventory, not evidence that those obligations are satisfied.
- GitHub private vulnerability reporting is enabled and **Security → Report a
  vulnerability** works for a non-maintainer.
- The candidate workflow has read-only repository contents permission and cannot
  create tags or publish a GitHub release. External publication remains a
  separate human-approved action.
- Product notes explain any new destructive behavior, protocol break, security
  boundary change, or prerequisite change.
- Release notes record the exact Linux distribution/kernel, Podman version, and
  Chromium-based browser version used for packaged-path acceptance.

Legal attribution, external account settings, protected-environment approval,
and the irreversible decision to publish require the human repository owner.
Engineering owns the version proposal, evidence, artifacts, and stop decision.

## Candidate verification

Start from the exact clean commit intended for the tag. Update both workspace
package versions and `Cargo.lock`, then verify:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
node --check static/app.js
node --test tests/frontend-terminal.test.cjs
bash -n tests/smoke.sh
./tests/smoke.sh
```

The real smoke must use working rootless Podman and finish without a test
container or terminal proxy. A skipped or unavailable Podman run is not release
evidence.

Review `Cargo.lock` with a current Rust security-advisory database and record the
tool and database revision in the candidate evidence. An unresolved advisory
that affects the packaged path is a stop condition. This check is not yet
automated by the candidate workflow and must not be silently omitted.

Build the release artifact only from that commit with `Cargo.lock` enforced. The
manual candidate workflow may upload CI artifacts for verification, but it must
not create a tag or GitHub release. Then, in a clean temporary directory:

1. verify the archive against the candidate `SHA256SUMS` before extraction;
2. inspect the archive for exactly the documented root and contents;
3. confirm `clannon --version` exactly matches the workspace version and the
   intended tag without its leading `v`;
4. run `clannon doctor` as an unprivileged user and require every mandatory
   check to pass;
5. launch the packaged executable, open its complete printed private URL in a
   supported Chromium-based Linux browser, and exercise create, PTY input and
   Ctrl-C, resize, refresh-sampled Activity, reconnect, and destroy;
6. stop with Ctrl-C, confirm graceful exit, and confirm no release-test container
   or terminal proxy remains.

The packaged executable—not only `cargo run` or `target/debug/clannon`—must pass
the user-visible path. Keep the release as a draft if the artifact target,
archive layout, checksum name, or supported libc baseline is not explicit.

## Publish and verify

After all gates and human preflight items pass, create the annotated tag from the
verified commit and create a draft GitHub release. Attach the exact archive and
`SHA256SUMS` already verified from that commit; do not rebuild after tagging.
Before publishing the draft, independently download both files from GitHub,
verify the checksum, and repeat the version and doctor checks. Publish release
notes with the supported host, Podman prerequisite, install/upgrade/remove link,
security policy, and all known alpha limits.

After publication, test the README commands verbatim from a clean user account.
If a release is unsafe, preserve its tag and artifacts as evidence, mark it
unsupported in the release notes, and publish a new patch version. Never repair
a published version by replacing files under the same tag.
