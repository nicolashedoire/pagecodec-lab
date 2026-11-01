#!/usr/bin/env python3
"""Compile and verify the integrated AArch64 no_std guest's RX instructions.

Only compiles; never runs the guest code on the host. The produced .hex is the
raw .text segment; the VM harness wraps it in the project's restricted ELF.
Use the Rust toolchain documented for the guest; output can vary by version.
"""
import argparse
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parent


def compiled_text(path):
    data=path.read_bytes()
    if data[:6]!=b'\x7fELF\x02\x01' or struct.unpack_from('<HH',data,16)!=(1,183):
        raise ValueError('expected AArch64 ELF64 relocatable object')
    off=struct.unpack_from('<Q',data,40)[0]
    width,count,names_index=struct.unpack_from('<HHH',data,58)
    if width!=64 or names_index>=count or off+width*count>len(data):
        raise ValueError('invalid section table')
    sections=[struct.unpack_from('<IIQQQQIIQQ',data,off+i*width) for i in range(count)]
    def payload(section):
        start,length=section[4],section[5]
        if start+length>len(data): raise ValueError('section out of bounds')
        return data[start:start+length]
    names=payload(sections[names_index])
    def name(section):
        start=section[0];end=names.find(b'\0',start)
        if end<0:raise ValueError('invalid section name')
        return names[start:end]
    matches=[(i,section) for i,section in enumerate(sections) if name(section)==b'.text']
    if len(matches)!=1:raise ValueError('expected one text section')
    index,section=matches[0]
    if any(s[1] in (4,9) and s[7]==index and s[5] for s in sections):
        raise ValueError('text contains relocations')
    code=payload(section)
    if not 392<len(code)<=4096 or len(code)%4:
        raise ValueError('expected more than 512 ELF bytes and at most one RX page')
    symbols=[s for s in sections if s[1]==2]
    if len(symbols)!=1:raise ValueError('expected one symbol table')
    sym=symbols[0]
    if sym[9]!=24 or sym[6]>=len(sections):raise ValueError('invalid symbol table')
    names=payload(sections[sym[6]])
    found=[]
    for i in range(0,sym[5],sym[9]):
        name_off,_,_,shndx,value,size=struct.unpack_from('<IBBHQQ',data,sym[4]+i)
        end=names.find(b'\0',name_off)
        if end<0:raise ValueError('invalid symbol name')
        if names[name_off:end]==b'_start':found.append((shndx,value,size))
    if found!=[(index,0,len(code))]:raise ValueError('entry is not the complete text section at offset zero')
    return code.hex()+'\n'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc',default='rustc')
    parser.add_argument('--write',action='store_true')
    args=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='guest-rust-') as temporary:
        obj=Path(temporary)/'rust-no-std.o'
        subprocess.run([args.rustc,'--target','aarch64-unknown-none-softfloat',
            '--edition','2021','--crate-type','bin','-C','panic=abort','-C','opt-level=2',
            '-C','relocation-model=static','--emit=obj='+str(obj),str(ROOT/'rust-no-std.rs')],check=True)
        encoded=compiled_text(obj)
    target=ROOT/'rust-no-std.hex'
    if args.write:target.write_text(encoded)
    elif target.read_text()!=encoded:raise SystemExit('Rust fixture differs; check compiler version')
    print(f'rust-no-std: {len(encoded.strip())//2} RX bytes')

if __name__=='__main__':main()
