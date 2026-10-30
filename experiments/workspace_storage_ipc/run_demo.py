#!/usr/bin/env python3
"""QEMU FIFO-requested synthetic snapshot save and startup restore."""
import argparse,json,tempfile,subprocess,time
from pathlib import Path
from vm_harness import Guest,SIZE,create,sha,fnv

def validate_slot(slot,generation,snapshot):
 if slot[:8]!=b'GSLOT001' or int.from_bytes(slot[8:16],'little')!=generation or slot[24]!=0xa5:raise RuntimeError('bad slot header')
 n=int.from_bytes(slot[16:20],'little')
 if not 0<n<=3584 or fnv(slot[8:20]+slot[512:512+n])!=int.from_bytes(slot[20:24],'little'):raise RuntimeError('bad slot checksum')
 if json.loads(slot[512:512+n])!=snapshot or any(slot[25:512]) or any(slot[512+n:]):raise RuntimeError('bad slot payload/padding')

def main():
 p=argparse.ArgumentParser();p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern');p.add_argument('--output-dir',required=True);a=p.parse_args()
 out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('output directory must be empty')
 guest=Path(a.guest);h=sha(guest.read_bytes());qemu=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
 blank={'base':{'title':'base','common':'seed'},'human':{'title':'base','common':'seed'},'assistant':{'title':'base','common':'seed'}}
 final={'base':blank['base'],'human':{'title':'base','common':'seed','theme':'human-two'},'assistant':{'title':'base','common':'seed','layout':'assistant'}}
 traces=[];states=[]
 with tempfile.TemporaryDirectory(prefix='workspace-storage-ipc-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE));first_image=None
  for index in (1,2):
   g=Guest(a.qemu,guest,disk,tmp,index,out,a.transport)
   try:
    for _ in range(8):
     ipc=g.call('ipc_status',{})
     if ipc.get('finished') is True:break
     time.sleep(.1)
    else:raise RuntimeError('FIFO did not finish: '+str(ipc))
    if any(ipc.get(k)!=[1,2,3,4] for k in ('accepted','applied','consumed')) or not ipc.get('save_denied') or ipc.get('queue_len')!=0:raise RuntimeError('IPC save not validated: '+str(ipc))
    storage=g.call('storage_status',{})
    expected={'restore':'blank' if index==1 else 'restored','restore_generation':None if index==1 else 1,'before':blank,'after':blank if index==1 else final,'save':{'slot':index-1,'generation':index,'snapshot':final}}
    if storage!=expected:raise RuntimeError('restore/save mismatch: '+str(storage))
    if g.call('document_snapshot',{})!=final:raise RuntimeError('bad document state')
    merge=g.call('document_merge',{});fields={x['key']:x for x in merge['fields']}
    if merge['conflicts'] or fields['theme']['origin']!='human' or fields['layout']['origin']!='assistant':raise RuntimeError('provenance mismatch')
    for tool in ('document_edit','document_load','document_save'):
     g.next_id+=1;req={'jsonrpc':'2.0','id':g.next_id,'method':'tools/call','params':{'name':tool,'arguments':{}}}
     g.stream.write(json.dumps(req,separators=(',',':')).encode()+b'\n');res=json.loads(g.stream.readline());g.trace.append({'request':req,'response':res})
     if res.get('id')!=g.next_id or res.get('error',{}).get('code')!=-32602:raise RuntimeError('unrestricted MCP mutation route')
    if g.call('document_snapshot',{})!=final:raise RuntimeError('rejected MCP changed copies')
    g.progress('save and startup restore');traces.append(g.trace);states.append(storage)
   finally:g.close()
   data=disk.read_bytes();validate_slot(data[4*512:12*512],1,final)
   if index==1:
    if data[:4*512]!=bytes(4*512) or data[12*512:]!=bytes(SIZE-12*512):raise RuntimeError('out-of-slot writes')
    first_image=data
   else:
    validate_slot(data[12*512:20*512],2,final)
    if data[:12*512]!=first_image[:12*512] or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('second write changed other sectors')
 if sha(guest.read_bytes())!=h:raise RuntimeError('image changed')
 (out/'workspace-storage-ipc-evidence.json').write_text(json.dumps({'guest_sha256':h,'qemu_version':qemu,'transport':a.transport,'states':states,'sessions':traces},indent=2)+'\n')
 print('PASS QEMU SVC FIFO save and startup restore')
if __name__=='__main__':main()
