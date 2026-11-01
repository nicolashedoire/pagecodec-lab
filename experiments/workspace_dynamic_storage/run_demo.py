#!/usr/bin/env python3
"""QEMU FIFO-requested synthetic snapshot save and startup restore."""
import argparse,json,tempfile,subprocess,time
from pathlib import Path
from vm_harness import Guest,SIZE,create,sha,fnv

def validate_slot(slot,generation,snapshot,files):
 if slot[:8]!=b'GSLOT001' or int.from_bytes(slot[8:16],'little')!=generation or slot[24]!=0xa5:raise RuntimeError('bad slot header')
 n=int.from_bytes(slot[16:20],'little')
 if not 0<n<=3584 or fnv(slot[8:20]+slot[512:512+n])!=int.from_bytes(slot[20:24],'little'):raise RuntimeError('bad slot checksum')
 if json.loads(slot[512:512+n])!={'format':'workspace-files-v1','document':snapshot,'files':files} or any(slot[25:512]) or any(slot[512+n:]):raise RuntimeError('bad slot payload/padding')

def legacy_slot(generation,snapshot):
 payload=json.dumps(snapshot,separators=(',',':')).encode();slot=bytearray(4096)
 slot[:8]=b'GSLOT001';slot[8:16]=generation.to_bytes(8,'little');slot[16:20]=len(payload).to_bytes(4,'little');slot[512:512+len(payload)]=payload
 slot[20:24]=fnv(slot[8:20]+payload).to_bytes(4,'little');slot[24]=0xa5
 return bytes(slot)

def reject(g,tool,args,message):
 g.call(tool,args,rejected=True)
 result=g.trace[-1]['response']['result']
 if result.get('content')!=[{'type':'text','text':message}]:raise RuntimeError('wrong refusal: '+str(result))

def main():
 p=argparse.ArgumentParser();p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern');p.add_argument('--output-dir',required=True);a=p.parse_args()
 out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('output directory must be empty')
 guest=Path(a.guest);h=sha(guest.read_bytes());qemu=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
 blank={'base':{'title':'base','common':'seed'},'human':{'title':'base','common':'seed'},'assistant':{'title':'base','common':'seed'}}
 final={'base':blank['base'],'human':{'title':'base','common':'seed','theme':'human-two'},'assistant':{'title':'base','common':'seed','layout':'assistant'}}
 traces=[];states=[];qmp_events=[];file_hex=b'persisted through recovery'.hex()
 with tempfile.TemporaryDirectory(prefix='workspace-storage-ipc-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE));first_image=None
  for index in (1,2,3,4):
   if index==3:
    # Simulate a damaged newest snapshot while the guest is stopped. The
    # following boot must use the intact older slot, then repair redundancy.
    damaged=bytearray(disk.read_bytes());damaged[12*512+512]^=1;disk.write_bytes(damaged)
   if index==4:
    # An old-format committed image must restore through the same service.
    disk=Path(tmp)/'legacy.raw';old=bytearray(SIZE);old[4*512:12*512]=legacy_slot(1,final);create(disk,old)
   g=Guest(a.qemu,guest,disk,tmp,index,out,a.transport)
   try:
    for _ in range(8):
     ipc=g.call('ipc_status',{})
     if ipc.get('finished') is True and ipc.get('consumed')==[1,2,3,4,6,7]:break
     time.sleep(.1)
    else:raise RuntimeError('FIFO did not finish: '+str(ipc))
    if not ipc.get('initial_finished') or not ipc.get('file_finished') or any(ipc.get(k)!=[1,2,3,4,6,7] for k in ('accepted','applied','consumed')) or not ipc.get('save_denied') or not ipc.get('denial_consumed') or ipc.get('queue_len')!=0 or not ipc.get('full_rejected') or not ipc.get('full_consumed') or any(ipc.get(k)!=[True,True] for k in ('range_rejected','overflow_rejected','duplicate_rejected','utf8_rejected','spoof_rejected')) or ipc.get('source')!='synthetic descriptors and bytes constructed by EL0':raise RuntimeError('IPC save and rejection invariants failed: '+str(ipc))
    g.progress('EL0 file put/get and process exit')
    storage=g.call('storage_status',{})
    expected={'restore':'blank' if index==1 else 'restored','restore_generation':None if index==1 else 5 if index in (2,3) else 1,'restore_slots':(['blank','blank'] if index==1 else ['valid','valid'] if index==2 else ['valid','invalid'] if index==3 else ['valid','blank']),'before':blank,'after':blank if index==1 else final,'save':{'slot':0 if index==1 else 1,'generation':1 if index==1 else 6 if index in (2,3) else 2,'readback_verified':True,'snapshot':final}}
    if storage!=expected:raise RuntimeError('restore/save mismatch: '+str(storage))
    if g.call('file_get',{'name':'note'})!={'name':'note','hex':b'hello'.hex()}:raise RuntimeError('EL0 file missing')
    if index==1:
     if g.call('file_list',{})!=[{'name':'note','size':5}]:raise RuntimeError('initial EL0 file list mismatch')
     if g.call('file_put',{'name':'notes.txt','data':'persisted through recovery'})!={'slot':0,'generation':3,'readback_verified':True}:raise RuntimeError('file save failed')
     if g.call('file_put',{'name':'temp.txt','data':'discard'})!={'slot':1,'generation':4,'readback_verified':True}:raise RuntimeError('temporary file save failed')
     if g.call('file_list',{})!=[{'name':'note','size':5},{'name':'notes.txt','size':26},{'name':'temp.txt','size':7}]:raise RuntimeError('file list mismatch')
     if g.call('file_delete',{'name':'temp.txt'})!={'slot':0,'generation':5,'readback_verified':True}:raise RuntimeError('file delete failed')
     reject(g,'file_delete',{'name':'temp.txt'},'file not found')
     reject(g,'file_put',{'name':'../bad','data':'no'},'invalid filename')
     reject(g,'file_get',{'name':'x'*33},'invalid filename')
    if index in (1,2,3) and g.call('file_get',{'name':'notes.txt'})!={'name':'notes.txt','hex':file_hex}:raise RuntimeError('named file lost during restore')
    if index in (1,2,3) and g.call('file_list',{})!=[{'name':'note','size':5},{'name':'notes.txt','size':26}]:raise RuntimeError('deleted file returned or catalog changed')
    if index in (2,3):reject(g,'file_get',{'name':'temp.txt'},'file not found')
    if g.call('document_snapshot',{})!=final:raise RuntimeError('bad document state')
    merge=g.call('document_merge',{});fields={x['key']:x for x in merge['fields']}
    if merge['conflicts'] or fields['theme']['origin']!='human' or fields['layout']['origin']!='assistant':raise RuntimeError('provenance mismatch')
    for tool in ('document_edit','document_load','document_save'):
     g.next_id+=1;req={'jsonrpc':'2.0','id':g.next_id,'method':'tools/call','params':{'name':tool,'arguments':{}}}
     g.stream.write(json.dumps(req,separators=(',',':')).encode()+b'\n');res=json.loads(g.stream.readline());g.trace.append({'request':req,'response':res})
     if res.get('id')!=g.next_id or res.get('error',{}).get('code')!=-32602:raise RuntimeError('unrestricted MCP mutation route')
    if g.call('document_snapshot',{})!=final:raise RuntimeError('rejected MCP changed copies')
    if index==1:
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
     if g.call('document_snapshot',{})!=final:raise RuntimeError('input changed saved document')
     qmp_events=g.qmp_trace
    g.progress('save, restore and input');traces.append(g.trace);states.append(storage)
   finally:g.close()
   data=disk.read_bytes()
   if index==1:
    validate_slot(data[4*512:12*512],5,final,{'note':b'hello'.hex(),'notes.txt':file_hex})
    validate_slot(data[12*512:20*512],4,final,{'note':b'hello'.hex(),'notes.txt':file_hex,'temp.txt':b'discard'.hex()})
    if data[:4*512]!=bytes(4*512) or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('out-of-slot writes')
    first_image=data
   elif index in (2,3):
    validate_slot(data[12*512:20*512],6,final,{'note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:12*512]!=first_image[:12*512] or data[20*512:]!=first_image[20*512:]:raise RuntimeError('recovery changed other sectors')
   else:
    validate_slot(data[4*512:12*512],3,final,{'note':b'hello'.hex()})
    validate_slot(data[12*512:20*512],2,final,{})
    if data[:4*512]!=bytes(4*512) or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('migration changed unrelated sectors')
 if sha(guest.read_bytes())!=h:raise RuntimeError('image changed')
 (out/'workspace-dynamic-storage-evidence.json').write_text(json.dumps({'guest_sha256':h,'qemu_version':qemu,'transport':a.transport,'states':states,'qmp_input':qmp_events,'sessions':traces},indent=2)+'\n')
 print('PASS QEMU EL0 FIFO edits, file list/delete/refusals, restart, recovery and legacy migration')
if __name__=='__main__':main()
