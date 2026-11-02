#!/usr/bin/env python3
"""Assemble the EL0 FP/SIMD guest ELF; never execute it on the host."""
import argparse
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parent

def main():
 p=argparse.ArgumentParser(description=__doc__)
 p.add_argument('--clang',default='clang');p.add_argument('--rustc',default='rustc');p.add_argument('--write',action='store_true')
 a=p.parse_args()
 sysroot=Path(subprocess.check_output([a.rustc,'--print','sysroot'],text=True).strip())
 host=next(line.split(': ',1)[1] for line in subprocess.check_output([a.rustc,'-vV'],text=True).splitlines() if line.startswith('host: '))
 linker=sysroot/'lib'/'rustlib'/host/'bin'/'rust-lld'
 spec=importlib.util.spec_from_file_location('fixture_validator',ROOT/'rebuild-rust-two-segment.py')
 module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
 with tempfile.TemporaryDirectory(prefix='guest-fp-') as tmp:
  obj=Path(tmp)/'fp.o';out=Path(tmp)/'fp.elf'
  subprocess.run([a.clang,'-target','aarch64-none-elf','-c',str(ROOT/'fp-actors.S'),'-o',str(obj)],check=True)
  subprocess.run([str(linker),'-flavor','gnu','-T',str(ROOT/'rust-two-segment.ld'),'--build-id=none','-z','max-page-size=4096',str(obj),'-o',str(out)],check=True)
  encoded=module.validate(out.read_bytes())
 target=ROOT/'fp-actors.hex'
 if a.write:target.write_text(encoded)
 elif target.read_text()!=encoded:raise SystemExit('FP fixture differs; check assembler/toolchain')
 print('fp-actors:',len(encoded.strip())//2,'bytes')

if __name__=='__main__':main()
