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
 traces=[];states=[];qmp_events=[];file_hex=b'persisted through recovery'.hex();binary_hex='00ff80fe0041';program_hex=None;next_hex=None
 # Exact raw image shipped in the previous VFS snapshot format.
 legacy_raw='014000f9c2ed8dd282aeacf2028000f902ad8cd2828dadf2e20dc0f2029000f902000491020001f9820080d2020401f902800491020801f9a20080d2020c01f9c20080d2021c00f9021c40f9021001f9e20301aa030080d2012600d4212600d4612000d4022840f9820300b4031c40f9630002cae30300b5033840f9630040d2830300b5430840d2a30100b5039041f9a20080d2630002cac30200b5039441f902ad8cd2828dadf2e20dc0f2630002ca030200b5220080d202a801f9022840f9022c00f9033840f9033c00f9021c40f942040091021c00f9e20301aa030080d2a12000d4212400d473060091d7ffff17a1750fd4ffffff17'
 with tempfile.TemporaryDirectory(prefix='workspace-storage-ipc-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE));first_image=None
  for index in (1,2,3,4,5,6,7):
   if index==3:
    # Simulate a damaged newest snapshot while the guest is stopped. The
    # following boot must use the intact older slot, then repair redundancy.
    damaged=bytearray(disk.read_bytes());damaged[4*512+512]^=1;disk.write_bytes(damaged)
   if index in (6,7):
    # An old-format committed image must restore through the same service.
    disk=Path(tmp)/('legacy'+str(index)+'.raw');old=bytearray(SIZE)
    old_doc=final if index==6 else {'format':'workspace-files-v1','document':final,'files':{'init.bin':legacy_raw,'keep.bin':'00ff'}}
    old[4*512:12*512]=legacy_slot(1,old_doc);create(disk,old)
   g=Guest(a.qemu,guest,disk,tmp,index,out,a.transport)
   try:
    if index==5:
     # A full catalog refuses the task's new file; it must exit cleanly.
     for _ in range(8):
      failed=g.call('el0_status',{});ipc=g.call('ipc_status',{})
      if failed.get('spawn_exited') is True:break
      time.sleep(.1)
     else:raise RuntimeError('failed file operation did not terminate task')
     if any(failed.get(k) is not True for k in ('spawn_admitted','spawn_failed','spawn_exited','alias_removed','frame_scrubbed','code_alias_removed','code_frame_scrubbed') if k not in ('alias_removed','frame_scrubbed')) or failed.get('alias_removed')!=[True,True] or failed.get('frame_scrubbed')!=[True,True] or failed.get('spawn_executed') is not False or failed.get('frame_pool_assigned')!=0:raise RuntimeError('failed task cleanup invalid: '+str(failed))
     if not ipc.get('initial_finished') or ipc.get('file_finished') or ipc.get('accepted')!=[1,2,3,4,6] or ipc.get('applied')!=[1,2,3,4,6] or ipc.get('queue_len')!=0:raise RuntimeError('failed IPC outcome invalid: '+str(ipc))
     expected_full=[{'name':'binary.bin','size':6},*[{'name':'fill'+str(n),'size':1} for n in range(4)],{'name':'init.bin','size':len(program_hex)//2},{'name':'next.bin','size':len(next_hex)//2},{'name':'notes.txt','size':26}]
     if g.call('file_list',{})!=expected_full:raise RuntimeError('failure fixture catalog differs from expected files')
     reject(g,'file_get',{'name':'note'},'file not found')
     stable=g.call('el0_status',{})
     if any(stable.get(k)!=failed.get(k) for k in ('spawn_exited','spawn_failed','frame_pool_assigned','code_alias_removed','code_frame_scrubbed','dispatches','slice_ticks')):raise RuntimeError('failed task was dispatched again')
     traces.append(g.trace);states.append(g.call('storage_status',{}));continue
    if index==4:
     # An existing but altered program must not be silently replaced or run.
     for _ in range(8):
      denied=g.call('el0_status',{});initial=g.call('ipc_status',{})
      if denied.get('loader_rejected') is True and denied.get('task_states')==['exited','exited'] and initial.get('initial_finished') is True:break
      time.sleep(.1)
     else:raise RuntimeError('altered loader image was not refused: '+str(denied))
     if denied.get('image_loaded') is not False or denied.get('spawn_executed') is not False or denied.get('frame_pool_assigned')!=0:raise RuntimeError('rejected image was admitted: '+str(denied))
     if g.call('file_get',{'name':'init.bin'})!={'name':'init.bin','hex':'ff'}:raise RuntimeError('altered image silently changed')
     if g.call('file_put_hex',{'name':'init.bin','hex':program_hex})!={'slot':1,'generation':10,'readback_verified':True}:raise RuntimeError('approved image repair failed')
    for _ in range(8):
     ipc=g.call('ipc_status',{})
     if ipc.get('finished') is True and ipc.get('consumed')==[1,2,3,4,6,7]:break
     time.sleep(.1)
    else:raise RuntimeError('FIFO did not finish: '+str(ipc))
    if not ipc.get('initial_finished') or not ipc.get('file_finished') or any(ipc.get(k)!=[1,2,3,4,6,7] for k in ('accepted','applied','consumed')) or not ipc.get('save_denied') or not ipc.get('denial_consumed') or ipc.get('queue_len')!=0 or not ipc.get('full_rejected') or not ipc.get('full_consumed') or any(ipc.get(k)!=[True,True] for k in ('range_rejected','overflow_rejected','duplicate_rejected','utf8_rejected','spoof_rejected')) or ipc.get('source')!='synthetic descriptors and bytes constructed by EL0':raise RuntimeError('IPC save and rejection invariants failed: '+str(ipc))
    g.progress('EL0 file put/get and process exit')
    storage=g.call('storage_status',{})
    expected={'restore':'blank' if index==1 else 'restored','restore_generation':None if index==1 else 6 if index in (2,3) else 8 if index==4 else 1,'restore_slots':(['blank','blank'] if index==1 else ['valid','valid'] if index in (2,4) else ['invalid','valid'] if index==3 else ['valid','blank']),'program_bootstrap':(['seeded','seeded'] if index in (1,6) else ['migrated-raw','seeded'] if index==7 else ['present','present']),'before':blank,'after':blank if index==1 else final,'save':{'slot':0 if index in (1,2,3,4) else 1,'generation':1 if index==1 else 7 if index in (2,3) else 9 if index==4 else 2,'readback_verified':True,'snapshot':final}}
    if storage!=expected:raise RuntimeError('restore/save mismatch: '+str(storage))
    init=g.call('file_get',{'name':'init.bin'})
    if init.get('name')!='init.bin' or not isinstance(init.get('hex'),str) or not init['hex'] or len(init['hex'])>1024:raise RuntimeError('invalid init program file')
    if program_hex is None:program_hex=init['hex']
    elif init['hex']!=program_hex:raise RuntimeError('init program changed during restore')
    second=g.call('file_get',{'name':'next.bin'})
    if second.get('name')!='next.bin' or not isinstance(second.get('hex'),str) or not second['hex'] or len(second['hex'])>1024:raise RuntimeError('invalid second program file')
    if next_hex is None:next_hex=second['hex']
    elif second['hex']!=next_hex:raise RuntimeError('second program changed during restore')
    if next_hex==program_hex:raise RuntimeError('program images unexpectedly identical')
    if index==7 and g.call('file_get',{'name':'keep.bin'})!={'name':'keep.bin','hex':'00ff'}:raise RuntimeError('migration changed unrelated legacy file')
    if index==6 and g.call('file_list',{})!=[{'name':'init.bin','size':len(program_hex)//2},{'name':'next.bin','size':len(next_hex)//2},{'name':'note','size':5}]:raise RuntimeError('unexpected files after document-only migration')
    if index==7 and g.call('file_list',{})!=[{'name':'init.bin','size':len(program_hex)//2},{'name':'keep.bin','size':2},{'name':'next.bin','size':len(next_hex)//2},{'name':'note','size':5}]:raise RuntimeError('unexpected files after raw-image migration')
    if g.call('file_get',{'name':'note'})!={'name':'note','hex':b'hello'.hex()}:raise RuntimeError('EL0 file missing')
    if index==1:
     if g.call('file_list',{})!=[{'name':'init.bin','size':len(program_hex)//2},{'name':'next.bin','size':len(next_hex)//2},{'name':'note','size':5}]:raise RuntimeError('initial EL0 file list mismatch')
     if g.call('file_put',{'name':'notes.txt','data':'persisted through recovery'})!={'slot':0,'generation':3,'readback_verified':True}:raise RuntimeError('file save failed')
     if g.call('file_put',{'name':'temp.txt','data':'discard'})!={'slot':1,'generation':4,'readback_verified':True}:raise RuntimeError('temporary file save failed')
     if g.call('file_list',{})!=[{'name':'init.bin','size':len(program_hex)//2},{'name':'next.bin','size':len(next_hex)//2},{'name':'note','size':5},{'name':'notes.txt','size':26},{'name':'temp.txt','size':7}]:raise RuntimeError('file list mismatch')
     if g.call('file_delete',{'name':'temp.txt'})!={'slot':0,'generation':5,'readback_verified':True}:raise RuntimeError('file delete failed')
     reject(g,'file_delete',{'name':'temp.txt'},'file not found')
     reject(g,'file_put',{'name':'../bad','data':'no'},'invalid filename')
     reject(g,'file_get',{'name':'x'*33},'invalid filename')
     reject(g,'file_delete',{'name':'init.bin'},'reserved file')
     reject(g,'file_delete',{'name':'next.bin'},'reserved file')
     reject(g,'file_put_hex',{'name':'binary.bin','hex':'f'},'hex length must be even')
     reject(g,'file_put_hex',{'name':'binary.bin','hex':'ffGG'},'invalid lowercase hexadecimal')
     if g.call('file_put_hex',{'name':'binary.bin','hex':binary_hex})!={'slot':1,'generation':6,'readback_verified':True}:raise RuntimeError('binary file save failed')
    if index in (1,2,3,4) and g.call('file_get',{'name':'notes.txt'})!={'name':'notes.txt','hex':file_hex}:raise RuntimeError('named file lost during restore')
    if index in (1,2,3,4) and g.call('file_get',{'name':'binary.bin'})!={'name':'binary.bin','hex':binary_hex}:raise RuntimeError('binary bytes lost during restore')
    if index in (1,2,3,4) and g.call('file_list',{})!=[{'name':'binary.bin','size':6},{'name':'init.bin','size':len(program_hex)//2},{'name':'next.bin','size':len(next_hex)//2},{'name':'note','size':5},{'name':'notes.txt','size':26}]:raise RuntimeError('deleted file returned or catalog changed')
    if index in (2,3,4):reject(g,'file_get',{'name':'temp.txt'},'file not found')
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
    g.progress('save, restore and input')
    if index==3 and g.call('file_put_hex',{'name':'init.bin','hex':'ff'})!={'slot':1,'generation':8,'readback_verified':True}:raise RuntimeError('altered image staging failed')
    if index==4:
     if g.call('file_delete',{'name':'note'})!={'slot':0,'generation':11,'readback_verified':True}:raise RuntimeError('failure fixture delete failed')
     for n in range(4):
      generation=12+n
      if g.call('file_put',{'name':'fill'+str(n),'data':'x'})!={'slot':1 if generation%2==0 else 0,'generation':generation,'readback_verified':True}:raise RuntimeError('failure fixture fill failed')
    traces.append(g.trace);states.append(storage)
   finally:g.close()
   data=disk.read_bytes()
   if index==1:
    validate_slot(data[4*512:12*512],5,final,{'init.bin':program_hex,'next.bin':next_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    validate_slot(data[12*512:20*512],6,final,{'binary.bin':binary_hex,'init.bin':program_hex,'next.bin':next_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:4*512]!=bytes(4*512) or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('out-of-slot writes')
    first_image=data
   elif index==2:
    validate_slot(data[4*512:12*512],7,final,{'binary.bin':binary_hex,'init.bin':program_hex,'next.bin':next_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:4*512]!=first_image[:4*512] or data[12*512:]!=first_image[12*512:]:raise RuntimeError('recovery changed other sectors')
   elif index==3:
    validate_slot(data[4*512:12*512],7,final,{'binary.bin':binary_hex,'init.bin':program_hex,'next.bin':next_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    validate_slot(data[12*512:20*512],8,final,{'binary.bin':binary_hex,'init.bin':'ff','next.bin':next_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:4*512]!=first_image[:4*512] or data[20*512:]!=first_image[20*512:]:raise RuntimeError('altered image staging changed unrelated sectors')
   elif index==4:
    filled={'binary.bin':binary_hex,'init.bin':program_hex,'next.bin':next_hex,'notes.txt':file_hex,**{'fill'+str(n):b'x'.hex() for n in range(4)}}
    validate_slot(data[4*512:12*512],15,final,filled)
   else:
    original={'keep.bin':'00ff'} if index==7 else {}
    validate_slot(data[4*512:12*512],3,final,{**original,'init.bin':program_hex,'next.bin':next_hex,'note':b'hello'.hex()})
    validate_slot(data[12*512:20*512],2,final,{**original,'init.bin':program_hex,'next.bin':next_hex})
    if data[:4*512]!=bytes(4*512) or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('migration changed unrelated sectors')
 if sha(guest.read_bytes())!=h:raise RuntimeError('image changed')
 (out/'workspace-dynamic-storage-evidence.json').write_text(json.dumps({'guest_sha256':h,'qemu_version':qemu,'transport':a.transport,'states':states,'qmp_input':qmp_events,'sessions':traces},indent=2)+'\n')
 print('PASS QEMU EL0 FIFO edits, binary file persistence, loader refusal/retry, recovery and legacy migration')
if __name__=='__main__':main()
