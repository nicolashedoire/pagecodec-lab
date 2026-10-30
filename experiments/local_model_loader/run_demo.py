#!/usr/bin/env python3
"""Load a synthetic bounded model and corrupted variants in isolated QEMU guests."""
import argparse, hashlib, json, os, selectors, subprocess, tempfile, time
from pathlib import Path

W1=[[3,-2,1],[-1,4,-1],[2,1,2],[-2,-1,3]]; B1=[-50,20,-100,30]
W2=[[2,-1,1,-1],[-1,1,2,1]]; B2=[10,-20]
def sha(data):return hashlib.sha256(data).hexdigest()
def pack8(v):return int(v).to_bytes(1,'little',signed=True)
def pack16(v):return int(v).to_bytes(2,'little',signed=True)
def reference(x):
    hidden=[max(0,sum(a*b for a,b in zip(row,x))+bias) for row,bias in zip(W1,B1)]
    return hidden,[sum(a*b for a,b in zip(row,hidden))+bias for row,bias in zip(W2,B2)]
def fixture():
    data=bytearray(512);data[:8]=b'NSDENSE1';data[8:10]=(1).to_bytes(2,'little')
    for offset,n in zip((12,16,20,24),(3,4,2,32)):data[offset:offset+4]=n.to_bytes(4,'little')
    payload=b''.join(pack8(x) for row in W1 for x in row)+b''.join(pack16(x) for x in B1)+b''.join(pack8(x) for row in W2 for x in row)+b''.join(pack16(x) for x in B2)
    assert len(payload)==32;data[60:92]=payload;data[28:60]=hashlib.sha256(data[:28]+payload).digest();return data
def create(path,data):
    fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
    with os.fdopen(fd,'wb') as f:f.write(data)
def boot(qemu,guest,image,expected,legacy):
    filedev={'driver':'file','filename':str(image),'node-name':'file0','read-only':True}
    rawdev={'driver':'raw','file':'file0','node-name':'disk0','read-only':True}
    cmd=[qemu,'-M','virt,virtualization=off','-accel','tcg','-cpu','cortex-a57','-smp','1','-m','128M',
         '-nodefaults','-display','none','-nic','none','-monitor','none','-serial','stdio','-no-reboot',
         '-kernel',str(guest),'-global','virtio-mmio.force-legacy='+('true' if legacy else 'false'),
         '-blockdev',json.dumps(filedev),'-blockdev',json.dumps(rawdev),'-device','virtio-blk-device,drive=disk0']
    proc=subprocess.Popen(cmd,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);out=bytearray()
    try:
        with selectors.DefaultSelector() as sel:
            sel.register(proc.stdout,selectors.EVENT_READ);deadline=time.monotonic()+30
            while time.monotonic()<deadline:
                if sel.select(.2):
                    out.extend(os.read(proc.stdout.fileno(),4096))
                    if any(line.startswith((b'PASS ',b'FAIL ',b'REJECT ')) for line in out.split(b'\n')[:-1]):break
                if proc.poll() is not None:raise RuntimeError('QEMU exited: '+out.decode(errors='replace'))
            else:raise TimeoutError('guest timeout: '+out.decode(errors='replace'))
    finally:
        proc.terminate()
        try:proc.wait(timeout=5)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
    serial=out.decode('ascii',errors='replace').replace('\r','')
    if serial.splitlines()!=expected:raise RuntimeError('unexpected guest output: '+serial)
    return serial

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--output-dir',required=True);p.add_argument('--legacy',action='store_true');args=p.parse_args()
    guest=Path(args.guest);guest_hash=sha(guest.read_bytes());version=subprocess.run([args.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
    expected_reference=[([0,20,0,30],[-40,30]),([250,0,100,0],[610,-70]),([0,420,0,0],[-410,400]),([150,220,400,30],[460,880])]
    assert [reference(x) for x in ([0,0,0],[100,0,0],[0,100,0],[100,100,100])]==expected_reference
    good=fixture();cases={}
    def variant(name,offset,payload,marker):
        d=good.copy();d[offset:offset+len(payload)]=payload;cases[name]=(d,['REJECT '+marker])
    cases['valid']=(good,['CHECK sha256=abc,boundaries shape=3x4x2 payload=32 input=bounded','CHECK cases=4 hidden_and_scores=reference guest_writes=none','SCORES zero=-40,30 cpu=610,-70 load=-410,400 all=460,880','PASS synthetic model load and inference'])
    variant('magic',0,b'X','magic');variant('version',8,b'\x02\x00','version');variant('flags',10,b'\x01\x00','version')
    variant('shape',12,(4).to_bytes(4,'little'),'shape');variant('overflow',12,(2**32-1).to_bytes(4,'little')*3,'overflow')
    variant('length31',24,(31).to_bytes(4,'little'),'length');variant('length513',24,(513).to_bytes(4,'little'),'length')
    variant('hash',28,bytes([good[28]^1]),'hash');variant('payload_corrupt',60,bytes([good[60]^1]),'hash');variant('padding',92,b'\x01','padding')
    logs={}
    with tempfile.TemporaryDirectory(prefix='local-model-') as tmp:
        for name,(sector,expected) in cases.items():
            path=Path(tmp)/(name+'.raw');image=bytes(sector)+bytes(1024*1024-512);create(path,image)
            logs[name]=boot(args.qemu,guest,path,expected,args.legacy)
            if path.read_bytes()!=image:raise RuntimeError('read-only image changed: '+name)
    if sha(guest.read_bytes())!=guest_hash:raise RuntimeError('guest image changed during run')
    result={'guest_sha256':guest_hash,'qemu_version':version,'transport':'legacy' if args.legacy else 'modern','fixture_sector_sha256':sha(good),'reference':expected_reference,'serial':logs,'limitations':'synthetic fixed model; SHA-256 is not authentication; no trained model, Jev, Laya or system control'}
    out=Path(args.output_dir);out.mkdir(parents=True,exist_ok=True);(out/'local-model-evidence.json').write_text(json.dumps(result,indent=2)+'\n')
    print('PASS model loading, inference and malformed cases in QEMU')
if __name__=='__main__':main()
