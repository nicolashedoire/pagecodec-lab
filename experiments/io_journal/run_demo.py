#!/usr/bin/env python3
"""Run synthetic journal persistence and malformed-record trials in QEMU."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time

SIZE=1024*1024
OFFSET=4*512

def sha(data):return hashlib.sha256(data).hexdigest()
def fnv(data):
    h=2166136261
    for b in data:h=((h^b)*16777619)&0xffffffff
    return h
def create(path,data):
    fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
    with os.fdopen(fd,'wb') as f:f.write(data)
def mode(name,length):return name+length.to_bytes(2,'little')+bytes(502)
def expected_record():
    record=bytearray(512);record[:8]=b'IOJRNL01';record[8]=16
    for i in range(16):
        off=16+i*8;record[off:off+4]=(i+1).to_bytes(4,'little')
        record[off+4]=i%4+1;record[off+5]=i%3+1;record[off+6]=i%3+1
    record[508:512]=fnv(record[:508]).to_bytes(4,'little')
    return bytes(record)
def boot(qemu,guest,image,marker,legacy):
    blocks=[{'driver':'file','filename':str(image),'node-name':'file0'}, {'driver':'raw','file':'file0','node-name':'disk0'}]
    cmd=[qemu,'-M','virt,virtualization=off','-accel','tcg','-cpu','cortex-a57','-smp','1','-m','128M',
         '-nodefaults','-display','none','-nic','none','-monitor','none','-serial','stdio','-no-reboot',
         '-kernel',str(guest),'-global','virtio-mmio.force-legacy='+('true' if legacy else 'false'),
         '-blockdev',json.dumps(blocks[0]),'-blockdev',json.dumps(blocks[1]),'-device','virtio-blk-device,drive=disk0']
    process=subprocess.Popen(cmd,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);out=bytearray()
    try:
        with selectors.DefaultSelector() as sel:
            sel.register(process.stdout,selectors.EVENT_READ);deadline=time.monotonic()+30
            while time.monotonic()<deadline:
                if sel.select(.2):
                    out.extend(os.read(process.stdout.fileno(),4096))
                    if any(line.startswith((b'PASS ',b'FAIL ',b'REJECT ')) for line in out.split(b'\n')[:-1]):break
                if process.poll() is not None:raise RuntimeError('QEMU exited: '+out.decode(errors='replace'))
            else:raise TimeoutError('guest timeout: '+out.decode(errors='replace'))
    finally:
        process.terminate()
        try:process.wait(timeout=5)
        except subprocess.TimeoutExpired:process.kill();process.wait()
    serial=out.decode('ascii',errors='replace').replace('\r','')
    if marker not in serial.splitlines():raise RuntimeError('unexpected guest output: '+serial)
    return serial
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--output-dir',required=True);p.add_argument('--legacy',action='store_true')
    args=p.parse_args();guest=Path(args.guest);guest_hash=sha(guest.read_bytes())
    version=subprocess.run([args.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
    logs={};expected=expected_record()
    with tempfile.TemporaryDirectory(prefix='io-journal-') as tmp:
        base=bytearray(SIZE);base[:512]=mode(b'IOINIT01',0);image=Path(tmp)/'journal.raw';create(image,base)
        logs['save']=boot(args.qemu,guest,image,'PASS save synthetic journal',args.legacy)
        saved=image.read_bytes();wanted=bytearray(base);wanted[OFFSET:OFFSET+512]=expected
        if saved!=wanted:raise RuntimeError('save differs outside expected synthetic sector')
        load=bytearray(saved);load[:512]=mode(b'IOLOAD01',512)
        with image.open('r+b') as f:f.write(load[:512])
        logs['replay']=boot(args.qemu,guest,image,'PASS replay synthetic journal',args.legacy)
        if image.read_bytes()!=load:raise RuntimeError('replay modified image')
        bad=bytearray(load);bad[OFFSET+20]^=1
        truncated=bytearray(load);truncated[:512]=mode(b'IOLOAD01',511)
        count=bytearray(load);count[OFFSET+8]=17;count[OFFSET+508:OFFSET+512]=fnv(count[OFFSET:OFFSET+508]).to_bytes(4,'little')
        sequence=bytearray(load);sequence[OFFSET+24:OFFSET+28]=(1).to_bytes(4,'little');sequence[OFFSET+508:OFFSET+512]=fnv(sequence[OFFSET:OFFSET+508]).to_bytes(4,'little')
        for name,data,marker in [('checksum',bad,'REJECT checksum'),('logical_truncation',truncated,'REJECT length'),('count',count,'REJECT count'),('sequence',sequence,'REJECT malformed')]:
            path=Path(tmp)/(name+'.raw');create(path,data);logs[name]=boot(args.qemu,guest,path,marker,args.legacy)
            if path.read_bytes()!=data:raise RuntimeError('rejected fixture was modified: '+name)
    if sha(guest.read_bytes())!=guest_hash:raise RuntimeError('guest image changed during run')
    result={'guest_sha256':guest_hash,'qemu_version':version,'transport':'legacy' if args.legacy else 'modern',
            'record_sha256':sha(expected),'record_bytes':512,'synthetic_event_count':16,'serial':logs,
            'truncation':'logical input slice of 511 bytes; raw disk remains full sized',
            'limitations':'FNV is not authentication; no power-loss durability; no real I/O monitoring'}
    out=Path(args.output_dir);out.mkdir(parents=True,exist_ok=True)
    (out/'io-journal-evidence.json').write_text(json.dumps(result,indent=2)+'\n')
    print('PASS journal save replay capacity and malformed fixtures in QEMU')
if __name__=='__main__':main()
