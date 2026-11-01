#!/usr/bin/env python3
"""Reproduce the AArch64 code bytes used by the integrated VM scenario.

By default verifies the checked-in hex; use --write to update it. Only
assembles source; it does not execute the generated guest instructions.
"""
import argparse
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent


def text_from_elf(path):
    data = path.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected little-endian ELF64 relocatable object")
    shoff = struct.unpack_from("<Q", data, 40)[0]
    entsize, count, names_index = struct.unpack_from("<HHH", data, 58)
    if entsize != 64 or names_index >= count:
        raise ValueError("invalid section table")
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * entsize) for i in range(count)]
    names_section = sections[names_index]
    names = data[names_section[4]:names_section[4]+names_section[5]]
    def name(section):
        start=section[0]; end=names.find(b"\0",start)
        if end<0: raise ValueError("invalid section name")
        return names[start:end]
    matches = [(i,s) for i,s in enumerate(sections) if name(s)==b".text"]
    if len(matches)!=1:
        raise ValueError("expected one .text section")
    index, section = matches[0]
    if any(s[1] in (4,9) and s[7]==index and s[5] for s in sections):
        raise ValueError("guest code contains unresolved relocations")
    code = data[section[4]:section[4]+section[5]]
    if not code or len(code)>392 or len(code)%4:
        raise ValueError("guest code exceeds the loader's 512-byte ELF bound")
    return code.hex()+"\n"


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clang",default="clang")
    parser.add_argument("--write",action="store_true")
    args=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="guest-fixtures-") as tmp:
        for stem in ("generic-read","generic-oversize"):
            obj=Path(tmp)/(stem+".o")
            subprocess.run([args.clang,"-target","aarch64-none-elf","-c",str(ROOT/(stem+".S")),"-o",str(obj)],check=True)
            encoded=text_from_elf(obj)
            target=ROOT/(stem+".hex")
            if args.write: target.write_text(encoded)
            elif target.read_text()!=encoded: raise SystemExit(f"fixture mismatch: {stem}")
            print(f"{stem}: {len(encoded.strip())//2} bytes")


if __name__=="__main__": main()
