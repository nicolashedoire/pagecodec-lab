#!/usr/bin/env python3
"""QEMU-only two-task FIFO document submission and MCP observation."""
import argparse,json,tempfile,subprocess,time
from pathlib import Path
from vm_harness import Guest,SIZE,create,sha

def main():
 p=argparse.ArgumentParser();p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern');p.add_argument('--output-dir',required=True);a=p.parse_args()
 out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('output directory must be empty')
 guest=Path(a.guest);h=sha(guest.read_bytes());qemu=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
 with tempfile.TemporaryDirectory(prefix='workspace-queue-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE));g=Guest(a.qemu,guest,disk,tmp,1,out,a.transport)
  try:
   for _ in range(8):
    state=g.call('ipc_status',{})
    if state.get('finished') is True:break
    time.sleep(.1)
   else:raise RuntimeError('EL0 scripts did not finish: '+str(state))
   if state.get('capacity')!=2 or state.get('queue_len')!=0 or state.get('accepted')!=[1,2,3] or state.get('applied')!=[1,2,3] or state.get('consumed')!=[1,2,3] or not state.get('full_rejected') or not state.get('full_consumed') or state.get('range_rejected')!=[True,True] or state.get('spoof_rejected')!=[True,True]:raise RuntimeError('FIFO or rejection invariant: '+str(state))
   snapshot=g.call('document_snapshot',{})
   expected={'base':{'title':'base','common':'seed'},'human':{'title':'base','common':'seed','theme':'human-two'},'assistant':{'title':'base','common':'seed','layout':'assistant'}}
   if snapshot!=expected:raise RuntimeError('document copies incorrect: '+str(snapshot))
   merged=g.call('document_merge',{});fields={v['key']:v for v in merged['fields']}
   if merged['conflicts'] or fields['theme']['origin']!='human' or fields['layout']['origin']!='assistant':raise RuntimeError('origin mismatch '+str(merged))
   for tool,args in [('document_edit',{'actor':'human','space':'human-space','key':'theme','value':'forged'}),('document_load',{}),('document_save',{'mode':'commit'})]:
    g.next_id+=1;req={'jsonrpc':'2.0','id':g.next_id,'method':'tools/call','params':{'name':tool,'arguments':args}}
    g.stream.write(json.dumps(req,separators=(',',':')).encode()+b'\n');res=json.loads(g.stream.readline());g.trace.append({'request':req,'response':res})
    if res.get('id')!=g.next_id or res.get('error',{}).get('code')!=-32602:raise RuntimeError('unexpected editable document route '+str(res))
   if g.call('document_snapshot',{})!=snapshot:raise RuntimeError('unauthorized MCP route changed document')
   g.input([{'type':'abs','data':{'axis':'x','value':8500}},{'type':'abs','data':{'axis':'y','value':24575}}])
   for _ in range(5):
    if g.pointer()=='human=114,444; ai=560,330':break
    time.sleep(.2)
   else:raise RuntimeError('synthetic pointer did not move')
   g.input([{'type':'btn','data':{'button':'left','down':True}}])
   g.wait_interaction({'human':{'focus':'action','clicks':1,'pressed':True},'assistant':{'focus':'none','clicks':0}})
   g.input([{'type':'btn','data':{'button':'left','down':False}}])
   g.wait_interaction({'human':{'focus':'action','clicks':1,'pressed':False},'assistant':{'focus':'none','clicks':0}})
   if g.call_text('move_ai',{'x':660,'y':444})!='moved AI pointer to 660,444':raise RuntimeError('AI move failed')
   if g.call('click_ai',{})!={'focus':'select','clicks':1}:raise RuntimeError('AI click failed')
   if g.call('document_snapshot',{})!=snapshot:raise RuntimeError('input changed document')
   g.progress('FIFO and QMP/MCP');trace=g.trace;qmp=g.qmp_trace
  finally:g.close()
  if disk.read_bytes()!=bytes(SIZE):raise RuntimeError('disk unexpectedly modified')
 if sha(guest.read_bytes())!=h:raise RuntimeError('guest changed during test')
 (out/'workspace-queue-vm-evidence.json').write_text(json.dumps({'guest_sha256':h,'qemu_version':qemu,'transport':a.transport,'status':state,'snapshot':snapshot,'qmp_input':qmp,'sessions':[trace]},indent=2)+'\n')
 print('PASS QEMU EL0 FIFO and MCP observation')
if __name__=='__main__':main()
