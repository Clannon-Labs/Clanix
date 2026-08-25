#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target=${1:-x86_64-unknown-linux-musl}
output_directory=${2:-"$repository_root/dist"}
binary=${CLANNON_RELEASE_BINARY:-"$repository_root/target/$target/release/clannon"}

version=$(python3 - "$repository_root/Cargo.toml" <<'PY'
import pathlib
import re
import sys

manifest = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
match = re.search(r"(?ms)^\[workspace\.package\].*?^version\s*=\s*\"([^\"]+)\"", manifest)
if match is None:
    raise SystemExit("could not read workspace.package.version")
print(match.group(1))
PY
)
tag="v$version"
bundle="clannon-$tag-$target"

for required in README.md PROJECT.md SECURITY.md LICENSE-MIT LICENSE-APACHE Cargo.lock; do
    if [[ ! -f "$repository_root/$required" ]]; then
        echo "release packaging requires $required" >&2
        exit 1
    fi
done
if [[ -n "$(git -C "$repository_root" status --porcelain --untracked-files=normal)" ]]; then
    echo "release packaging requires a clean Git worktree" >&2
    exit 1
fi
if [[ ! -x "$binary" ]]; then
    echo "release binary is missing or not executable: $binary" >&2
    exit 1
fi
if [[ "$($binary --version)" != "clannon $version" ]]; then
    echo "release binary version does not match workspace version $version" >&2
    exit 1
fi

temporary_directory=$(mktemp -d)
trap 'rm -rf "$temporary_directory"' EXIT
bundle_root="$temporary_directory/$bundle"
mkdir -p "$bundle_root" "$output_directory"
chmod 0755 "$bundle_root"

install -m 0755 "$binary" "$bundle_root/clannon"
install -m 0644 "$repository_root/README.md" "$bundle_root/README.md"
install -m 0644 "$repository_root/PROJECT.md" "$bundle_root/PROJECT.md"
install -m 0644 "$repository_root/SECURITY.md" "$bundle_root/SECURITY.md"
install -m 0644 "$repository_root/LICENSE-MIT" "$bundle_root/LICENSE-MIT"
install -m 0644 "$repository_root/LICENSE-APACHE" "$bundle_root/LICENSE-APACHE"

(
    cd "$repository_root"
    cargo tree --locked --workspace --edges normal --prefix none
) | sed -E \
    -e 's/ \(\*\)$//' \
    -e 's# \([^)]*/crates/(runtime|server)\)$##' \
    | LC_ALL=C sort -u > "$bundle_root/DEPENDENCIES.txt"

commit=$(git -C "$repository_root" rev-parse HEAD)
lock_sha256=$(sha256sum "$repository_root/Cargo.lock" | cut -d' ' -f1)
binary_sha256=$(sha256sum "$binary" | cut -d' ' -f1)
cat > "$bundle_root/BUILD-INFO.txt" <<EOF
Clannon version: $version
Git commit: $commit
Rust target: $target
Cargo.lock SHA-256: $lock_sha256
Binary SHA-256: $binary_sha256
Rust compiler: $(rustc --version)
EOF

source_date_epoch=${SOURCE_DATE_EPOCH:-$(git -C "$repository_root" show -s --format=%ct HEAD)}
archive="$output_directory/$bundle.tar.gz"
tar \
    --sort=name \
    --mtime="@$source_date_epoch" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    -C "$temporary_directory" \
    -cf - "$bundle" | gzip -n > "$archive"

(
    cd "$output_directory"
    sha256sum "$(basename "$archive")" > SHA256SUMS
)

printf '%s\n' "$archive"
