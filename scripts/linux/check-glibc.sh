#!/usr/bin/env bash
# 打印 ELF 文件要求的最高 GLIBC 符号版本，超过下限时失败。
# 用法：check-glibc.sh [--max 2.35] <ELF 文件或目录>...
# 目录会递归检查其中的 ELF 文件（例如解包后的 .deb）。
set -euo pipefail

max_version="${NERI_GLIBC_MAX:-2.35}"
if [[ "${1:-}" == "--max" ]]; then
  max_version="$2"
  shift 2
fi
if (( $# == 0 )); then
  echo "usage: $0 [--max 2.35] <elf-or-dir>..." >&2
  exit 2
fi

command -v objdump >/dev/null || { echo "objdump not found (install binutils)" >&2; exit 2; }

is_elf() {
  [[ -f "$1" ]] && [[ "$(head -c 4 "$1" | od -An -c | tr -d ' \n')" == '177ELF' ]]
}

files=()
for path in "$@"; do
  if [[ -d "$path" ]]; then
    while IFS= read -r -d '' candidate; do
      is_elf "$candidate" && files+=("$candidate")
    done < <(find "$path" -type f -print0)
  elif is_elf "$path"; then
    files+=("$path")
  else
    echo "not an ELF file: $path" >&2
    exit 2
  fi
done
(( ${#files[@]} > 0 )) || { echo "no ELF files found in: $*" >&2; exit 2; }

overall=""
failed=0
for file in "${files[@]}"; do
  # 只看动态符号表里引用的版本；sort -V 正确比较 2.9 < 2.17 < 2.35
  required="$(objdump -T "$file" 2>/dev/null | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n 1 || true)"
  if [[ -z "$required" ]]; then
    echo "  -       $file"
    continue
  fi
  highest="$(printf '%s\n%s\n' "$required" "$max_version" | sort -V | tail -n 1)"
  if [[ "$highest" != "$max_version" ]]; then
    echo "  FAIL    GLIBC_$required  $file" >&2
    failed=1
  else
    echo "  ok      GLIBC_$required  $file"
  fi
  overall="$(printf '%s\n%s\n' "${overall:-0}" "$required" | sort -V | tail -n 1)"
done

echo "max GLIBC required: ${overall:-none} (limit $max_version, ${#files[@]} files)"
if (( failed )); then
  echo "::error title=glibc floor exceeded::binary requires glibc newer than $max_version; build inside ubuntu:22.04 (scripts/linux/build-in-container.sh)" >&2
  exit 1
fi
