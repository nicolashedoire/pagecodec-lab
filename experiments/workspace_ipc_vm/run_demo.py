#!/usr/bin/env python3
import argparse,json,tempfile,subprocess
from pathlib import Path
from vm_harness import Guest,SIZE,create,sha

def main():
 p=argparse.ArgumentParser();p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern');p.add_argument('--output-dir',required=True);a=p.parse_args()
 out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('output directory must be empty')
 guest=Path(a.guest);h=sha(guest.read_bytes());qemu=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True).stdout.splitlines()[0]
 with tempfile.TemporaryDirectory(prefix='workspace-ipc-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE));g=Guest(a.qemu,guest,disk,tmp,1,out,a.transport)
  try:
   state=g.call('ipc_status',{})
   if state!={'slots':[31,31],'responses':[[4,2,3,3],[2,4,3,3]],'payloads':[41377,45746],'oversize_values':[1342177280,1342177280],'capacity':2,'protocol':'fixed-svc-opcodes'}:raise RuntimeError('unexpected mailbox '+str(state))
   snap=g.call('document_snapshot',{});expected={'base':{'title':'base','common':'seed'},'human':{'title':'base','common':'seed','slot-a1a1':'el0-a1a1'},'assistant':{'title':'base','common':'seed','slot-b2b2':'el0-b2b2'}}
   if snap!=expected:raise RuntimeError('invalid copies '+str(snap))
   merged=g.call('document_merge',{});fields={f['key']:f for f in merged['fields']}
   if merged['conflicts'] or fields['slot-a1a1']['origin']!='human' or fields['slot-b2b2']['origin']!='assistant':raise RuntimeError('bad provenance '+str(merged))
   for tool,args in [('document_edit',{'actor':'human','space':'human-space','key':'from-b','value':'forged'}),('document_load',{}),('document_save',{'mode':'commit'})]:
    g.next_id+=1;req={'jsonrpc':'2.0','id':g.next_id,'method':'tools/call','params':{'name':tool,'arguments':args}}
    g.stream.write(json.dumps(req,separators=(',',':')).encode()+b'\n');res=json.loads(g.stream.readline());g.trace.append({'request':req,'response':res})
    if res.get('id')!=g.next_id or res.get('error',{}).get('code')!=-32602:raise RuntimeError('mutation route not rejected '+str(res))
   if g.call('document_snapshot',{})!=snap or g.call('ipc_status',{})!=state:raise RuntimeError('rejected access mutated state')
   g.progress('EL0 IPC and MCP reads');sessions=g.trace
  finally:g.close()
  if disk.read_bytes()!=bytes(SIZE):raise RuntimeError('disk unexpectedly modified')
 if sha(guest.read_bytes())!=h:raise RuntimeError('image changed')
 (out/'workspace-ipc-vm-evidence.json').write_text(json.dumps({'guest_sha256':h,'qemu_version':qemu,'transport':a.transport,'snapshot':snap,'mailboxes':state,'sessions':[sessions]},indent=2)+'\n')
 print('PASS fixed SVC workspace submission and read-only MCP')
if __name__=='__main__':main()
