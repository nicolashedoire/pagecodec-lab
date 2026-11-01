#!/usr/bin/env python3
"""Expect a synthetic injected x5 corruption to take the IRQ diagnostic path."""
import argparse,hashlib,json,subprocess,tempfile
from pathlib import Path
from vm_harness import Guest,SIZE,create

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--output-dir',required=True);a=p.parse_args()
 out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('output directory must be empty')
 guest=Path(a.guest);before=hashlib.sha256(guest.read_bytes()).hexdigest()
 version=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
 with tempfile.TemporaryDirectory(prefix='irq-diag-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE))
  try:g=Guest(a.qemu,guest,disk,tmp,1,out,'modern')
  except RuntimeError as e:
   message=str(e)
   for part in ('FAIL IRQ family=2','current=','ttbr=','elr=','spsr=','user_sp=','x0=','x1=','x2=','x3=','x4=','x5=42','x30='):
    if part not in message:raise RuntimeError('missing diagnostic field '+part+': '+message)
  else:
   g.close();raise RuntimeError('fault-injected guest unexpectedly passed')
 if hashlib.sha256(guest.read_bytes()).hexdigest()!=before:raise RuntimeError('guest changed')
 (out/'irq-diagnostic-evidence.json').write_text(json.dumps({'guest_sha256':before,'qemu_version':version,'injected_fault':'EL0 saved x5=42 before slices','expected':'register-check family 2','observed':message},indent=2)+'\n')
 print('PASS injected IRQ diagnostic with saved frame')
if __name__=='__main__':main()
