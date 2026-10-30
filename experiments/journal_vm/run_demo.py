#!/usr/bin/env python3
"""Exercise bounded inference-event logging and synthetic disk replay in QEMU."""
import argparse,json,subprocess,tempfile
from pathlib import Path
from regression import Guest, SIZE, create, sha, fnv


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True)
    p.add_argument('--output-dir',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern')
    a=p.parse_args(); out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('output directory must be empty')
    guest=Path(a.guest); guest_hash=sha(guest.read_bytes())
    version=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
    sessions=[]
    with tempfile.TemporaryDirectory(prefix='journal-vm-') as tmp:
        image=Path(tmp)/'disk.raw';create(image,bytes(SIZE))
        one=Guest(a.qemu,guest,image,tmp,1,out,a.transport)
        try:
            snapshot=one.call('document_snapshot',{});pointer=one.pointer();focus=one.call('interaction_state',{})
            if one.call('journal_status',{})!={'entries':[],'dropped':0}:raise RuntimeError('journal not empty')
            outcomes=['ok','rejected','ok','rejected']+['ok']*12
            for outcome in outcomes:
                if outcome=='ok':
                    result=one.call('infer_synthetic',{'pressure':20,'cpu_load':40,'io_activity':60})
                    if result!={'model_version':'local-inference-sketch-0.1.0-handwritten-v1','scores':[-120,410]}:raise RuntimeError('bad inference')
                else:one.call('infer_synthetic',{'pressure':101,'cpu_load':0,'io_activity':0},rejected=True)
            entries=[{'sequence':i+1,'category':'input','actor':'system','outcome':kind} for i,kind in enumerate(outcomes)]
            if one.call('journal_status',{})!={'entries':entries,'dropped':0}:raise RuntimeError('incorrect recorded results')
            # Saturation never suppresses or fakes the real request result.
            for valid in (True,False,True):
                if valid:
                    r=one.call('infer_synthetic',{'pressure':0,'cpu_load':0,'io_activity':0})
                    if r['scores']!=[-40,30]:raise RuntimeError('saturated success changed')
                else:one.call('infer_synthetic',{'pressure':-1,'cpu_load':0,'io_activity':0},rejected=True)
            if one.call('journal_status',{})!={'entries':entries,'dropped':3}:raise RuntimeError('overflow not explicit')
            if one.call('journal_save',{})!={'operation':'save','entries':16,'dropped_in_memory':3}:raise RuntimeError('save failed')
            if one.call('document_snapshot',{})!=snapshot or one.pointer()!=pointer or one.call('interaction_state',{})!=focus:raise RuntimeError('logging changed guest state')
            one.progress('journal inference/save');sessions.append(one.trace)
        finally:one.close()
        saved=image.read_bytes();sector=saved[20*512:21*512]
        expected=bytearray(512);expected[:8]=b'IOJRNL01';expected[8]=16
        for i,kind in enumerate(outcomes):
            j=16+8*i;expected[j:j+4]=(i+1).to_bytes(4,'little');expected[j+4:j+7]=bytes((4,3,1 if kind=='ok' else 2))
        expected[508:512]=fnv(expected[:508]).to_bytes(4,'little')
        if sector!=expected or saved[:20*512]!=bytes(20*512) or saved[21*512:]!=bytes(SIZE-21*512):raise RuntimeError('unexpected disk content')
        two=Guest(a.qemu,guest,image,tmp,2,out,a.transport)
        try:
            if two.call('journal_status',{})!={'entries':[],'dropped':0}:raise RuntimeError('boot journal not empty')
            if two.call('journal_load',{})!={'operation':'load','entries':16,'dropped_in_memory':0}:raise RuntimeError('load failed')
            if two.call('journal_status',{})!={'entries':entries,'dropped':0}:raise RuntimeError('replay differs')
            # Reloaded journal remains full; input still executes and reports dropped.
            result=two.call('infer_synthetic',{'pressure':0,'cpu_load':0,'io_activity':0})
            if result['scores']!=[-40,30] or two.call('journal_status',{})!={'entries':entries,'dropped':1}:raise RuntimeError('post-reload saturation')
            two.progress('journal recovery');sessions.append(two.trace)
        finally:two.close()
        if image.read_bytes()!=saved:raise RuntimeError('read/replay changed image')
        corrupt=bytearray(saved);corrupt[20*512+16]^=1;image.write_bytes(corrupt)
        three=Guest(a.qemu,guest,image,tmp,3,out,a.transport)
        try:
            three.call('infer_synthetic',{'pressure':0,'cpu_load':0,'io_activity':0})
            before=three.call('journal_status',{});three.call('journal_load',{},rejected=True)
            if three.call('journal_status',{})!=before:raise RuntimeError('bad load changed journal')
            sessions.append(three.trace)
        finally:three.close()
        if image.read_bytes()!=bytes(corrupt):raise RuntimeError('bad read changed image')
    if sha(guest.read_bytes())!=guest_hash:raise RuntimeError('guest changed during run')
    evidence={'guest_sha256':guest_hash,'qemu_version':version,'transport':a.transport,'journal_sector_sha256':sha(sector),'sessions':sessions,
              'limitations':'Synthetic inference handler results only; actor is unauthenticated system label; dropped count is RAM-only; no payload, delivery proof, universal I/O, authentication, atomicity or durability.'}
    (out/'journal-vm-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    print('PASS QEMU synthetic inference journal, saturation, restart and corrupt-sector rejection')
if __name__=='__main__':main()
