#!/usr/bin/env bash
set -euo pipefail
if [[ $# -ne 2 ]]; then
  echo "usage: $0 <downloaded-candidate> <private-output-root>" >&2
  exit 2
fi
source_file="$1"
root="$2"
pub_dir="$root/pubs"
layer_dir="$root/layers"
mkdir -p "$pub_dir" "$layer_dir"

cfb_magic="d0cf11e0a1b11ae1"
actual_magic="$(xxd -p -l 8 "$source_file" | tr '[:upper:]' '[:lower:]')"
if [[ "$actual_magic" == "$cfb_magic" ]]; then
  cp "$source_file" "$pub_dir/direct.pub"
  exit 0
fi

declare -A seen=()
queue="$(mktemp)"
next="$(mktemp)"
printf '%s\n' "$source_file" > "$queue"

for ((depth=0; depth<=4; depth++)); do
  : > "$next"
  while IFS= read -r container; do
    [[ -f "$container" ]] || continue
    digest="$(sha256sum "$container" | awk '{print $1}')"
    [[ -z "${seen[$digest]:-}" ]] || continue
    seen[$digest]=1

    if [[ "${container,,}" == *.mdf ]]; then
      converted="$layer_dir/converted-$depth-$digest.iso"
      if mdf2iso "$container" "$converted" >/dev/null 2>&1 && [[ -s "$converted" ]]; then
        echo "converted MDF depth=$depth sha256=$digest"
        printf '%s\n' "$converted" >> "$next"
      else
        echo "MDF conversion failed depth=$depth sha256=$digest" >&2
      fi
      continue
    fi

    if ! 7z l -ba "$container" >/dev/null 2>&1; then
      echo "skip non-7z-readable depth=$depth name=$(basename "$container")"
      continue
    fi

    echo "inspect layer depth=$depth sha256=$digest name=$(basename "$container")"
    pub_out="$pub_dir/depth-$depth-$digest"
    mkdir -p "$pub_out"
    7z x "$container" "-o$pub_out" -y -r '*.pub' '*.PUB' >/dev/null 2>&1 || true
    if ! find "$pub_out" -type f -iname '*.pub' -print -quit | grep -q .; then
      rmdir "$pub_out" 2>/dev/null || true
    fi

    if (( depth < 4 )); then
      nested="$layer_dir/depth-$depth-$digest"
      mkdir -p "$nested"
      7z x "$container" "-o$nested" -y -r         '*.puz' '*.PUZ' '*.cab' '*.CAB' '*.zip' '*.ZIP' '*.7z' '*.rar' '*.RAR'         '*.exe' '*.EXE' '*.iso' '*.ISO' '*.img' '*.IMG' '*.mdf' '*.MDF' '*.msi' '*.MSI'         >/dev/null 2>&1 || true
      while IFS= read -r child; do
        printf '%s\n' "$child" >> "$next"
      done < <(find "$nested" -type f \(         -iname '*.puz' -o -iname '*.cab' -o -iname '*.zip' -o -iname '*.7z' -o         -iname '*.rar' -o -iname '*.exe' -o -iname '*.iso' -o -iname '*.img' -o         -iname '*.mdf' -o -iname '*.msi' \) -print)
    fi
  done < "$queue"
  [[ -s "$next" ]] || break
  cp "$next" "$queue"
done

rm -f "$queue" "$next"
count="$(find "$pub_dir" -type f -iname '*.pub' | wc -l | tr -d ' ')"
if [[ "$count" -eq 0 ]]; then
  echo "no PUB member recovered" >&2
  exit 3
fi
echo "recovered_pub_candidates=$count"
