#!/usr/bin/env python3
"""Compile the actual static AArch64 Rust ELF; never execute it on the host."""
import argparse
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parent

def validate(data,fixture=None):
 if data[:7]!=b'\x7fELF\x02\x01\x01' or data[7] not in (0,3) or len(data)>16384:
  raise ValueError('unexpected ELF identification or size')
 if struct.unpack_from('<HH',data,16)!=(2,183):raise ValueError('expected static AArch64 executable')
 phoff=struct.unpack_from('<Q',data,32)[0];phentsize,phnum=struct.unpack_from('<HH',data,54)
 if phoff!=64 or (phentsize,phnum)!=(56,2):raise ValueError('expected exactly two program headers')
 segments=[struct.unpack_from('<IIQQQQQQ',data,phoff+i*phentsize) for i in range(2)]
 rx=[p for p in segments if p[0]==1 and p[1]==5 and p[3]==0x50001000]
 rw=[p for p in segments if p[0]==1 and p[1]==6 and p[3]==0x50000000]
 if len(rx)!=1 or len(rw)!=1:raise ValueError('expected one RX and one RW load segment')
 for p in segments:
  _,_,off,va,_,filesz,memsz,align=p
  if filesz>memsz or memsz>4096 or off+filesz>len(data) or align==0 or off%align!=va%align:raise ValueError('bad segment bounds/alignment')
 expected_rw = rw[0][5]>=8 if fixture=='rust-global-allocator' else rw[0][5]==8 and rw[0][6]>rw[0][5]
 if not expected_rw or data[rw[0][2]:rw[0][2]+8]!=bytes.fromhex('8877665544332211'):
  raise ValueError('initialized data or BSS not present')
 shoff=struct.unpack_from('<Q',data,40)[0];shwidth,shcount=struct.unpack_from('<HH',data,58)
 if shwidth!=64 or shoff+shwidth*shcount>len(data):raise ValueError('bad section table')
 for i in range(shcount):
  at=shoff+i*shwidth;kind=struct.unpack_from('<I',data,at+4)[0];length=struct.unpack_from('<Q',data,at+32)[0]
  if kind in (4,9) and length:raise ValueError('relocations are not supported')
 return data.hex()+'\n'

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--rustc',default='rustc');p.add_argument('--write',action='store_true');p.add_argument('--fixture',choices=('rust-two-segment','rust-direct-ipc','rust-net-roundtrip','rust-ipc-queue','rust-ipc-wait','rust-ipc-timeout','rust-three-process','rust-ipc-three','rust-net-three','rust-ipc-multi','rust-document-process','rust-private-heap','rust-multipage-heap','rust-global-allocator','rust-chunk-io','rust-tree-read','rust-tree-write','rust-tree-dir','rust-tree-list','rust-tree-paths','rust-surface','rust-surface-key'),default='rust-two-segment');a=p.parse_args()
 sysroot=Path(subprocess.check_output([a.rustc,'--print','sysroot'],text=True).strip())
 host=next(line.split(': ',1)[1] for line in subprocess.check_output([a.rustc,'-vV'],text=True).splitlines() if line.startswith('host: '))
 linker=sysroot/'lib'/'rustlib'/host/'bin'/'rust-lld'
 with tempfile.TemporaryDirectory(prefix='guest-two-segment-') as tmp:
  output=Path(tmp)/'fixture.elf'
  lto=['-C','lto=fat','-C','codegen-units=1'] if a.fixture=='rust-global-allocator' else []
  opt='z' if a.fixture in ('rust-global-allocator','rust-tree-list','rust-tree-paths','rust-surface','rust-surface-key') else '2'
  script='rust-global-allocator.ld' if a.fixture=='rust-global-allocator' else 'rust-two-segment.ld'
  subprocess.run([a.rustc,'--target','aarch64-unknown-none-softfloat','--edition','2021','--crate-type','bin','-C','panic=abort','-C','opt-level='+opt,*lto,'-C','relocation-model=static','-C','linker='+str(linker),'-C','linker-flavor=ld.lld','-C','link-arg=-T'+str(ROOT/script),'-C','link-arg=--build-id=none','-C','link-arg=-z','-C','link-arg=max-page-size=4096','-C','strip=symbols',str(ROOT/(a.fixture+'.rs')),'-o',str(output)],check=True)
  text=validate(output.read_bytes(),a.fixture)
 target=ROOT/(a.fixture+'.hex')
 if a.write:target.write_text(text)
 elif target.read_text()!=text:raise SystemExit('linked fixture differs; check Rust toolchain')
 print(a.fixture+':',len(text.strip())//2,'bytes')

if __name__=='__main__':main()
