"""Reject redistributable/debug CRT imports in the standalone Windows package."""
import re
import sys
from pathlib import Path


def imports(data):
    def word(offset, size=4):
        if offset < 0 or offset + size > len(data):
            raise ValueError("Truncated PE header/table")
        return int.from_bytes(data[offset:offset + size], "little")

    pe = word(0x3c)
    if data[:2] != b"MZ" or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("Expected a Windows PE executable")
    optional = pe + 24
    magic = word(optional, 2)
    if magic not in (0x10b, 0x20b):
        raise ValueError("Unsupported PE optional header")
    directories = optional + (112 if magic == 0x20b else 96)
    count = word(directories - 4)
    base = word(optional + (24 if magic == 0x20b else 28), 8 if magic == 0x20b else 4)
    section = optional + word(pe + 20, 2)
    sections = []
    for i in range(word(pe + 6, 2)):
        at = section + i * 40
        sections.append((word(at + 12), word(at + 16), word(at + 20)))

    def offset(rva):
        for start, size, raw in sections:
            if start <= rva < start + size:
                return raw + rva - start
        raise ValueError("Import RVA lies outside a file-backed section")

    result = set()
    for directory, stride, name_at in [(1, 20, 12), (13, 32, 4)]:
        if directory >= count:
            continue
        rva = word(directories + directory * 8)
        length = word(directories + directory * 8 + 4)
        if not rva:
            continue
        table = offset(rva)
        for i in range(min(length // stride, 4096)):
            at = table + i * stride
            row = data[at:at + stride]
            if len(row) != stride:
                raise ValueError("Truncated import descriptor")
            if not any(row):
                break
            name = word(at + name_at)
            if directory == 13 and not word(at) & 1:
                name -= base
            begin = offset(name)
            end = data.find(b"\0", begin, begin + 260)
            if end < 0:
                raise ValueError("Invalid DLL import name")
            result.add(data[begin:end].decode("ascii"))
        else:
            raise ValueError("Unterminated import descriptors")
    return sorted(result)


def check(path):
    names = imports(Path(path).read_bytes())
    forbidden = [name for name in names if re.match(r"(?:vcruntime\d|msvcp\d|msvcr\d|ucrtbased\.)", name, re.I)]
    if forbidden:
        raise ValueError("Unexpected VC redistributable/debug runtime DLLs: " + ", ".join(forbidden))
    print("Verified PE runtime imports: " + ", ".join(names))


if __name__ == "__main__":
    check(sys.argv[1])
