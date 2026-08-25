#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 <archive.tar.gz> <SHA256SUMS>" >&2
    exit 2
fi

archive=$(realpath "$1")
checksums=$(realpath "$2")
archive_directory=$(dirname "$archive")
archive_name=$(basename "$archive")
bundle=${archive_name%.tar.gz}
version=${bundle#clannon-v}
version=${version%-x86_64-unknown-linux-musl}

if [[ $(wc -l < "$checksums") -ne 1 ]]; then
    echo "SHA256SUMS must contain exactly one entry" >&2
    exit 1
fi
read -r checksum_name checksum_file < "$checksums"
if [[ ! "$checksum_name" =~ ^[0-9a-f]{64}$ || "$checksum_file" != "$archive_name" ]]; then
    echo "SHA256SUMS does not name the requested archive exactly" >&2
    exit 1
fi
(
    cd "$archive_directory"
    sha256sum --check "$checksums"
)

temporary_directory=$(mktemp -d)
trap 'rm -rf "$temporary_directory"' EXIT
bundle_root="$temporary_directory/$bundle"

expected=$(printf '%s\n' \
    "$bundle/" \
    "$bundle/BUILD-INFO.txt" \
    "$bundle/DEPENDENCIES.txt" \
    "$bundle/LICENSE-APACHE" \
    "$bundle/LICENSE-MIT" \
    "$bundle/PROJECT.md" \
    "$bundle/README.md" \
    "$bundle/SECURITY.md" \
    "$bundle/clannon")
actual=$(tar -tzf "$archive")
if [[ "$actual" != "$expected" ]]; then
    echo "release archive contains an unexpected layout" >&2
    diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") || true
    exit 1
fi
expected_types=$(printf '%s\n' d - - - - - - - -)
actual_types=$(tar -tvzf "$archive" | awk '{ print substr($1, 1, 1) }')
if [[ "$actual_types" != "$expected_types" ]]; then
    echo "release archive contains a non-regular entry" >&2
    exit 1
fi
tar --extract --gzip --no-same-owner --no-same-permissions \
    --file "$archive" --directory "$temporary_directory"
for regular_file in \
    BUILD-INFO.txt DEPENDENCIES.txt LICENSE-APACHE LICENSE-MIT \
    PROJECT.md README.md SECURITY.md clannon; do
    if [[ ! -f "$bundle_root/$regular_file" || -L "$bundle_root/$regular_file" ]]; then
        echo "release entry is not a regular file: $regular_file" >&2
        exit 1
    fi
done
if [[ "$($bundle_root/clannon --version)" != "clannon $version" ]]; then
    echo "extracted binary version does not match archive name" >&2
    exit 1
fi
if readelf -l "$bundle_root/clannon" | grep -q "INTERP"; then
    echo "release binary is dynamically linked" >&2
    readelf -l "$bundle_root/clannon" >&2
    exit 1
fi

echo "verified $archive_name"
