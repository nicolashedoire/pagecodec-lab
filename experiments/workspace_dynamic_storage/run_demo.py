#!/usr/bin/env python3
"""QEMU FIFO-requested synthetic snapshot save and startup restore."""
import argparse,json,struct,tempfile,subprocess,time
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

def independent_elf(reference_hex,code):
 # Reuse only the bounded ELF header; instructions are supplied by the
 # launcher, persisted in VFS, and not linked into the kernel image.
 reference=bytes.fromhex(reference_hex)
 if len(reference)<120 or reference[:4]!=b'\x7fELF' or not code or len(code)%4 or len(code)>4096:raise RuntimeError('invalid external program template')
 elf=bytearray(reference[:120]);elf[96:104]=len(code).to_bytes(8,'little');elf[104:112]=len(code).to_bytes(8,'little')
 return (elf+code).hex()

def fnv64(data):
 value=0xcbf29ce484222325
 for byte in data:value=((value^byte)*0x100000001b3)&0xffffffffffffffff
 return f'{value:016x}'

def stage_program(g,image):
 upload_id=g.call('upload_begin',{'size':len(image),'checksum':fnv64(image)})['id']
 for offset in range(0,len(image),512):
  g.call('upload_chunk',{'id':upload_id,'offset':offset,'hex':image[offset:offset+512].hex()})
 if g.call('upload_seal',{'id':upload_id})!={'sealed':True}:raise RuntimeError('program fixture upload failed')
 return upload_id

def compact_two_segment(elf):
 # Repack the actual linked segments only for negative loader tests. Preserve
 # their virtual addresses and contents; remove unneeded section headers.
 if len(elf)<176 or struct.unpack_from('<H',elf,56)[0]!=2:raise RuntimeError('expected two-segment ELF')
 out=bytearray(elf[:176]);struct.pack_into('<Q',out,40,0);struct.pack_into('<HHH',out,58,0,0,0)
 for i in range(2):
  at=64+i*56;off=struct.unpack_from('<Q',elf,at+8)[0];size=struct.unpack_from('<Q',elf,at+32)[0]
  segment=elf[off:off+size]
  if len(segment)!=size:raise RuntimeError('bad fixture segment bounds')
  struct.pack_into('<Q',out,at+8,len(out));struct.pack_into('<Q',out,at+48,1);out.extend(segment)
 return bytes(out)

def wait_rust_program(g,admitted):
 state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','stored Rust ELF exited')
 if state.get('incarnations',[None,None])[0]!=admitted.get('incarnation') or state.get('outcomes',[None,None])[0]!=1 or state.get('exit_codes',[None,None])[0]!=0 or state.get('data_scrubbed',[False,False])[0] is not True or state.get('code_scrubbed',[False,False])[0] is not True or state.get('frame_pool_assigned')!=0:raise RuntimeError('stored Rust process exit or cleanup invalid: '+str(state))

def independent_exit_elf(reference_hex,exit_code):
 if not 0<=exit_code<=65535:raise RuntimeError('invalid external exit code')
 instructions=(0xd2800000 | (exit_code<<5),
               0xd4000001 | (0x123<<5))
 code=b''.join(i.to_bytes(4,'little') for i in instructions)
 return independent_elf(reference_hex,code)

def reject(g,tool,args,message):
 g.call(tool,args,rejected=True)
 result=g.trace[-1]['response']['result']
 if result.get('content')!=[{'type':'text','text':message}]:raise RuntimeError('wrong refusal: '+str(result))

def exercise_reclaiming_heap(g,preflight=False):
 # Both the allocator probe and the long run execute in the unified guest.
 probe=g.call('heap_exercise',{})
 if any(probe.get(k) is not True for k in ('coalesced','oom_reported','recovered','live_restored','alignments_ok')):raise RuntimeError('guest allocator probe failed: '+str(probe))
 if preflight:return [g.call('heap_status',{})]
 def request(i):
  if i%4==0:g.call('document_snapshot',{})
  elif i%4==1:g.call('file_list',{})
  elif i%4==2:
   if g.call('file_get',{'name':'note'})!={'name':'note','hex':b'hello'.hex()}:raise RuntimeError('file read failed during heap exercise')
  else:g.call('upload_status',{})
 for i in range(32):request(i)
 samples=[g.call('heap_status',{})]
 for batch in range(10):
  for i in range(32):request(batch*32+i)
  samples.append(g.call('heap_status',{}))
 start,end=samples[0],samples[-1]
 if any(not isinstance(s.get(k),int) for s in samples for k in ('capacity','live_bytes','peak_bytes','free_bytes','largest_free','alloc_attempts','alloc_successes','frees','active_allocations','metadata_exhaustions')):raise RuntimeError('invalid allocator counters')
 if any(s['capacity']!=start['capacity'] or s['metadata_exhaustions']!=0 for s in samples):raise RuntimeError('allocator capacity or metadata exhausted: '+str(samples))
 if end['alloc_successes']<=start['alloc_successes'] or end['frees']<=start['frees']:raise RuntimeError('no allocation and free activity observed')
 # The counters are captured while servicing the same heap_status request.
 # Small JSON bookkeeping variations are allowed; growth must not accumulate.
 if max(s['live_bytes'] for s in samples)-min(s['live_bytes'] for s in samples)>4096 or start['largest_free']-end['largest_free']>4096:raise RuntimeError('heap did not settle across repeated requests: '+str(samples))
 return samples

def expected_list(catalog):
 return [{'name':name,'size':len(hexbytes)//2} for name,hexbytes in sorted(catalog.items())]

def ipc_history_matches(ipc,base,consumed):
 accepted=ipc.get('accepted');applied=ipc.get('applied')
 return accepted in (base,base+[6],base+[6,6]) and applied in (base,base+[6],base+[6,6]) and len(applied)<=len(accepted) and ipc.get('consumed') in (consumed,consumed+[6])

def main():
 p=argparse.ArgumentParser();p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern');p.add_argument('--output-dir',required=True);p.add_argument('--exercise-storage-fault',action='store_true',help='require a guest built with storage-fault-test');p.add_argument('--heap-preflight',action='store_true',help='run the short heap probe while the guest still has its message limit');a=p.parse_args()
 out=Path(a.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('output directory must be empty')
 guest=Path(a.guest);h=sha(guest.read_bytes());qemu=subprocess.run([a.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
 blank={'base':{'title':'base','common':'seed'},'human':{'title':'base','common':'seed'},'assistant':{'title':'base','common':'seed'}}
 final={'base':blank['base'],'human':{'title':'base','common':'seed','theme':'human-two'},'assistant':{'title':'base','common':'seed','layout':'assistant'}}
 traces=[];states=[];qmp_events=[];file_hex=b'persisted through recovery'.hex();binary_hex='00ff80fe0041';program_hex=None;next_hex=None;extra_images={};loop_hex=None
 # Exact raw image shipped in the previous VFS snapshot format.
 legacy_raw='014000f9c2ed8dd282aeacf2028000f902ad8cd2828dadf2e20dc0f2029000f902000491020001f9820080d2020401f902800491020801f9a20080d2020c01f9c20080d2021c00f9021c40f9021001f9e20301aa030080d2012600d4212600d4612000d4022840f9820300b4031c40f9630002cae30300b5033840f9630040d2830300b5430840d2a30100b5039041f9a20080d2630002cac30200b5039441f902ad8cd2828dadf2e20dc0f2630002ca030200b5220080d202a801f9022840f9022c00f9033840f9033c00f9021c40f942040091021c00f9e20301aa030080d2a12000d4212400d473060091d7ffff17a1750fd4ffffff17'
 with tempfile.TemporaryDirectory(prefix='workspace-storage-ipc-') as tmp:
  disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE));first_image=None
  for index in (1,2,3,4,5,6,7,8,9):
   if index==3:
    # Simulate a damaged newest snapshot while the guest is stopped. The
    # following boot must use the intact older slot, then repair redundancy.
    damaged=bytearray(disk.read_bytes());damaged[4*512+512]^=1;disk.write_bytes(damaged)
   if index in (6,7,8):
    # An old-format committed image must restore through the same service.
    disk=Path(tmp)/('legacy'+str(index)+'.raw');old=bytearray(SIZE)
    old_doc=final if index==6 else {'format':'workspace-files-v1','document':final,'files':({'init.bin':legacy_raw,'keep.bin':'00ff'} if index==7 else {'init.bin':program_hex,'next.bin':next_hex,'keep.bin':'00ff'})}
    old[4*512:12*512]=legacy_slot(1,old_doc);create(disk,old)
   g=Guest(a.qemu,guest,disk,tmp,index,out,a.transport)
   try:
    if ('debug_fail_postcommit' in g.tools)!=a.exercise_storage_fault:raise RuntimeError('unexpected storage test failpoint in guest')
    if index==5:
     # A full catalog refuses the task's new file; it must exit cleanly.
     for _ in range(8):
      failed=g.call('el0_status',{});ipc=g.call('ipc_status',{})
      if failed.get('image_count')==4 and failed.get('images_reclaimed')==[True]*4:break
      time.sleep(.1)
     else:raise RuntimeError('failed file operation did not terminate task')
     if failed.get('image_outcomes')!=['file_error','fault','bad_svc','success'] or failed.get('images_reclaimed')!=[True]*4 or failed.get('completions_revoked')!=[True]*4 or failed.get('stale_response_blocked') is not True or failed.get('alias_removed')!=[True,True] or failed.get('frame_scrubbed')!=[True,True] or failed.get('code_alias_removed') is not True or failed.get('code_frame_scrubbed') is not True or failed.get('spawn_executed') is not True or failed.get('frame_pool_assigned')!=0:raise RuntimeError('failed task cleanup or later program invalid: '+str(failed))
     if not ipc.get('initial_finished') or ipc.get('file_finished') or not ipc_history_matches(ipc,[1,2,3,4,6],[1,2,3,4]) or ipc.get('consumed')!=[1,2,3,4,6] or failed.get('stale_generation_rejected') is not True or ipc.get('queue_len')!=0:raise RuntimeError('failed IPC outcome invalid: '+str(ipc))
     expected_full=expected_list({'binary.bin':binary_hex,'init.bin':program_hex,'next.bin':next_hex,**extra_images,'notes.txt':file_hex,**{'fill'+str(n):'78' for n in range(4)}})
     if g.call('file_list',{})!=expected_full:raise RuntimeError('failure fixture catalog differs from expected files')
     reject(g,'file_get',{'name':'note'},'file not found')
     stable=g.call('el0_status',{})
     if any(stable.get(k)!=failed.get(k) for k in ('image_outcomes','images_reclaimed','frame_pool_assigned','code_alias_removed','code_frame_scrubbed','dispatches','slice_ticks')):raise RuntimeError('failed task was dispatched again')
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
     if ipc.get('finished') is True and ipc.get('consumed') in ([1,2,3,4,6,7],[1,2,3,4,6,7,6]):break
     time.sleep(.1)
    else:raise RuntimeError('FIFO did not finish: '+str(ipc))
    if not ipc.get('initial_finished') or not ipc.get('file_finished') or not ipc_history_matches(ipc,[1,2,3,4,6,7],[1,2,3,4,6,7]) or not ipc.get('save_denied') or not ipc.get('denial_consumed') or ipc.get('queue_len')!=0 or not ipc.get('full_rejected') or not ipc.get('full_consumed') or any(ipc.get(k)!=[True,True] for k in ('range_rejected','overflow_rejected','duplicate_rejected','utf8_rejected','spoof_rejected')) or ipc.get('source')!='synthetic descriptors and bytes constructed by EL0':raise RuntimeError('IPC save and rejection invariants failed: '+str(ipc))
    g.progress('EL0 file put/get and process exit',timeout_image=index==9)
    completed=g.call('ipc_status',{})
    expected_consumed=[1,2,3,4,6,7] if index==9 else [1,2,3,4,6,7,6]
    if completed.get('consumed')!=expected_consumed:raise RuntimeError('later incarnation completion missing or duplicated: '+str(completed))
    storage=g.call('storage_status',{})
    expected={'restore':'blank' if index==1 else 'restored','restore_generation':None if index==1 else 6 if index in (2,3) else 8 if index==4 else 4 if index==9 else 1,'restore_slots':(['blank','blank'] if index==1 else ['valid','valid'] if index in (2,4,9) else ['invalid','valid'] if index==3 else ['valid','blank']),'program_bootstrap':(['seeded']*4 if index in (1,6) else ['migrated-raw','seeded','seeded','seeded'] if index==7 else ['present','seeded','seeded','present'] if index==8 else ['present']*4),'before':blank,'after':blank if index==1 else final,'save':{'slot':0 if index in (1,2,3,4,9) else 1,'generation':1 if index==1 else 7 if index in (2,3) else 9 if index==4 else 5 if index==9 else 2,'readback_verified':True,'snapshot':final},'write_blocked':False}
    if storage!=expected:raise RuntimeError('restore/save mismatch: '+str(storage))
    init=g.call('file_get',{'name':'init.bin'})
    if init.get('name')!='init.bin' or not isinstance(init.get('hex'),str) or not init['hex'] or len(init['hex'])>1024:raise RuntimeError('invalid init program file')
    if program_hex is None:program_hex=init['hex']
    elif init['hex']!=program_hex:raise RuntimeError('init program changed during restore')
    second=g.call('file_get',{'name':'next.bin'})
    if second.get('name')!='next.bin' or not isinstance(second.get('hex'),str) or not second['hex'] or len(second['hex'])>1024:raise RuntimeError('invalid second program file')
    if next_hex is None:next_hex=second['hex']
    elif second['hex']!=next_hex:raise RuntimeError('second program changed during restore')
    for name in ('fault.bin','badsvc.bin'):
     entry=g.call('file_get',{'name':name});hexbytes=entry.get('hex')
     if entry.get('name')!=name or not isinstance(hexbytes,str) or not hexbytes or len(hexbytes)>1024:raise RuntimeError('invalid fault program file')
     if name not in extra_images:extra_images[name]=hexbytes
     elif index==9 and name=='badsvc.bin' and hexbytes==loop_hex:pass
     elif extra_images[name]!=hexbytes:raise RuntimeError('fault program changed during restore')
    if len(set([program_hex,next_hex,*extra_images.values()]))!=4:raise RuntimeError('program images unexpectedly identical')
    images={'init.bin':program_hex,'next.bin':next_hex,**extra_images}
    if index==9:images['badsvc.bin']=loop_hex
    if loop_hex is None:
     # User controls x19.  A timer preempts the infinite loop after this
     # instruction; the next program must still be admitted after timeout.
     loop_hex=independent_elf(extra_images['badsvc.bin'],bytes.fromhex('1300809200000014')) # movn x19,#0; b .
    if index in (7,8) and g.call('file_get',{'name':'keep.bin'})!={'name':'keep.bin','hex':'00ff'}:raise RuntimeError('migration changed unrelated legacy file')
    if index==6 and g.call('file_list',{})!=expected_list({**images,'note':b'hello'.hex()}):raise RuntimeError('unexpected files after document-only migration')
    if index in (7,8) and g.call('file_list',{})!=expected_list({**images,'note':b'hello'.hex(),'keep.bin':'00ff'}):raise RuntimeError('unexpected files after snapshot migration')
    if index==9 and g.call('file_list',{})!=expected_list({**images,'note':b'hello'.hex(),'keep.bin':'00ff'}):raise RuntimeError('unexpected files after timeout restart')
    if g.call('file_get',{'name':'note'})!={'name':'note','hex':b'hello'.hex()}:raise RuntimeError('EL0 file missing')
    if index==1:
     if g.call('file_list',{})!=expected_list({**images,'note':b'hello'.hex()}):raise RuntimeError('initial EL0 file list mismatch')
     if g.call('file_put',{'name':'notes.txt','data':'persisted through recovery'})!={'slot':0,'generation':3,'readback_verified':True}:raise RuntimeError('file save failed')
     if g.call('file_put',{'name':'temp.txt','data':'discard'})!={'slot':1,'generation':4,'readback_verified':True}:raise RuntimeError('temporary file save failed')
     if g.call('file_list',{})!=expected_list({**images,'note':b'hello'.hex(),'notes.txt':file_hex,'temp.txt':b'discard'.hex()}):raise RuntimeError('file list mismatch')
     if g.call('file_delete',{'name':'temp.txt'})!={'slot':0,'generation':5,'readback_verified':True}:raise RuntimeError('file delete failed')
     reject(g,'file_delete',{'name':'temp.txt'},'file not found')
     reject(g,'file_put',{'name':'../bad','data':'no'},'invalid filename')
     reject(g,'file_get',{'name':'x'*33},'invalid filename')
     reject(g,'file_delete',{'name':'init.bin'},'reserved file')
     reject(g,'file_delete',{'name':'next.bin'},'reserved file')
     reject(g,'file_delete',{'name':'fault.bin'},'reserved file')
     reject(g,'file_delete',{'name':'badsvc.bin'},'reserved file')
     reject(g,'file_put_hex',{'name':'binary.bin','hex':'f'},'hex length must be even')
     reject(g,'file_put_hex',{'name':'binary.bin','hex':'ffGG'},'invalid lowercase hexadecimal')
     if g.call('file_put_hex',{'name':'binary.bin','hex':binary_hex})!={'slot':1,'generation':6,'readback_verified':True}:raise RuntimeError('binary file save failed')
    if index in (1,2,3,4) and g.call('file_get',{'name':'notes.txt'})!={'name':'notes.txt','hex':file_hex}:raise RuntimeError('named file lost during restore')
    if index in (1,2,3,4) and g.call('file_get',{'name':'binary.bin'})!={'name':'binary.bin','hex':binary_hex}:raise RuntimeError('binary bytes lost during restore')
    if index in (1,2,3,4) and g.call('file_list',{})!=expected_list({**images,'binary.bin':binary_hex,'note':b'hello'.hex(),'notes.txt':file_hex}):raise RuntimeError('deleted file returned or catalog changed')
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
    g.progress('save, restore and input',timeout_image=index==9)
    if index==3 and g.call('file_put_hex',{'name':'init.bin','hex':'ff'})!={'slot':1,'generation':8,'readback_verified':True}:raise RuntimeError('altered image staging failed')
    if index==4:
     if g.call('file_delete',{'name':'note'})!={'slot':0,'generation':11,'readback_verified':True}:raise RuntimeError('failure fixture delete failed')
     for n in range(4):
      generation=12+n
      if g.call('file_put',{'name':'fill'+str(n),'data':'x'})!={'slot':1 if generation%2==0 else 0,'generation':generation,'readback_verified':True}:raise RuntimeError('failure fixture fill failed')
    if index==8 and g.call('file_put_hex',{'name':'badsvc.bin','hex':loop_hex})!={'slot':1,'generation':4,'readback_verified':True}:raise RuntimeError('timeout image staging failed')
    traces.append(g.trace);states.append(storage)
   finally:g.close()
   data=disk.read_bytes()
   if index==1:
    validate_slot(data[4*512:12*512],5,final,{**images,'note':b'hello'.hex(),'notes.txt':file_hex})
    validate_slot(data[12*512:20*512],6,final,{**images,'binary.bin':binary_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:4*512]!=bytes(4*512) or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('out-of-slot writes')
    first_image=data
   elif index==2:
    validate_slot(data[4*512:12*512],7,final,{**images,'binary.bin':binary_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:4*512]!=first_image[:4*512] or data[12*512:]!=first_image[12*512:]:raise RuntimeError('recovery changed other sectors')
   elif index==3:
    validate_slot(data[4*512:12*512],7,final,{**images,'binary.bin':binary_hex,'note':b'hello'.hex(),'notes.txt':file_hex})
    validate_slot(data[12*512:20*512],8,final,{**images,'binary.bin':binary_hex,'init.bin':'ff','note':b'hello'.hex(),'notes.txt':file_hex})
    if data[:4*512]!=first_image[:4*512] or data[20*512:]!=first_image[20*512:]:raise RuntimeError('altered image staging changed unrelated sectors')
   elif index==4:
    filled={**images,'binary.bin':binary_hex,'notes.txt':file_hex,**{'fill'+str(n):b'x'.hex() for n in range(4)}}
    validate_slot(data[4*512:12*512],15,final,filled)
   elif index in (6,7):
    original={'keep.bin':'00ff'} if index in (7,8) else {}
    validate_slot(data[4*512:12*512],3,final,{**original,**images,'note':b'hello'.hex()})
    validate_slot(data[12*512:20*512],2,final,{**original,**images})
    if data[:4*512]!=bytes(4*512) or data[20*512:]!=bytes(SIZE-20*512):raise RuntimeError('migration changed unrelated sectors')
   elif index==8:
    validate_slot(data[4*512:12*512],3,final,{**images,'keep.bin':'00ff','note':b'hello'.hex()})
    validate_slot(data[12*512:20*512],4,final,{**images,'badsvc.bin':loop_hex,'keep.bin':'00ff','note':b'hello'.hex()})
   else:
    validate_slot(data[4*512:12*512],5,final,{**images,'keep.bin':'00ff','note':b'hello'.hex()})
    validate_slot(data[12*512:20*512],4,final,{**images,'keep.bin':'00ff','note':b'hello'.hex()})
  # Persist new executable bytes via the VFS, then restart on the same disk.
  # Two codes exercise both successful and unsuccessful task termination.
  for attempt,exit_code in enumerate((0,42)):
   external_hex=independent_exit_elf(next_hex,exit_code)
   g=Guest(a.qemu,guest,disk,tmp,13+attempt*2,out,a.transport)
   try:
    g.progress('before external program',timeout_image=True,expected_exit_code=0 if attempt else None)
    saved=g.call('file_put_hex',{'name':'next.bin','hex':external_hex})
    if saved.get('readback_verified') is not True:raise RuntimeError('external ELF was not saved')
    if g.call('file_get',{'name':'next.bin'})!={'name':'next.bin','hex':external_hex}:raise RuntimeError('external ELF differs before restart')
    traces.append(g.trace);states.append(g.call('storage_status',{}))
   finally:g.close()
   g=Guest(a.qemu,guest,disk,tmp,14+attempt*2,out,a.transport)
   try:
    if g.call('file_get',{'name':'next.bin'})!={'name':'next.bin','hex':external_hex}:raise RuntimeError('external ELF not restored')
    g.progress('external program execution',timeout_image=True,expected_exit_code=exit_code)
    if g.call('document_snapshot',{})!=final:raise RuntimeError('external ELF changed workspace')
    traces.append(g.trace);states.append(g.call('storage_status',{}))
   finally:g.close()
  # Two independently assembled programs exercise the generic file-read SVC.
  # Both are stored through VFS and executed only after a complete VM restart.
  # Their source and reproducible machine code are in fixtures/.
  for attempt,fixture in enumerate(('generic-read','generic-oversize')):
   code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/(fixture+'.hex')).read_text().strip())
   external_hex=independent_elf(next_hex,code)
   g=Guest(a.qemu,guest,disk,tmp,17+attempt*2,out,a.transport)
   try:
    g.progress('before generic file reader',timeout_image=True,expected_exit_code=42 if attempt==0 else 0)
    if attempt==0:
     if not g.call('file_put',{'name':'big','data':'x'*33}).get('readback_verified'):raise RuntimeError('oversize file was not saved')
    saved=g.call('file_put_hex',{'name':'next.bin','hex':external_hex})
    if saved.get('readback_verified') is not True:raise RuntimeError('generic reader ELF was not saved')
    if g.call('file_get',{'name':'next.bin'})!={'name':'next.bin','hex':external_hex}:raise RuntimeError('generic reader differs before restart')
    traces.append(g.trace);states.append(g.call('storage_status',{}))
   finally:g.close()
   g=Guest(a.qemu,guest,disk,tmp,18+attempt*2,out,a.transport)
   try:
    if g.call('file_get',{'name':'next.bin'})!={'name':'next.bin','hex':external_hex}:raise RuntimeError('generic reader ELF not restored')
    g.progress('generic file-reader execution',timeout_image=True,expected_exit_code=0)
    ipc=g.call('ipc_status',{});expected_reads=3 if attempt==0 else 2
    if ipc.get('generic_read_submitted')!=expected_reads or ipc.get('generic_read_completed')!=expected_reads:raise RuntimeError('generic file service did not complete expected reads: '+str(ipc))
    if g.call('file_get',{'name':'note'})!={'name':'note','hex':b'hello'.hex()} or g.call('file_get',{'name':'big'})!={'name':'big','hex':(b'x'*33).hex()}:raise RuntimeError('generic reader changed files')
    if g.call('document_snapshot',{})!=final:raise RuntimeError('generic reader changed workspace')
    traces.append(g.trace);states.append(g.call('storage_status',{}))
   finally:g.close()
  # The writer is another loaded ELF, not a host-side substitute. Fill the
  # catalog to nine entries after the first file it creates, then test ENOSPC.
  writer_code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'generic-write.hex').read_text().strip())
  writer_hex=independent_elf(next_hex,writer_code)
  g=Guest(a.qemu,guest,disk,tmp,21,out,a.transport)
  try:
   g.progress('before generic writer',timeout_image=True,expected_exit_code=0)
   if len(g.call('file_list',{}))!=7:raise RuntimeError('unexpected catalog before generic writer')
   if not g.call('file_put',{'name':'fill','data':'x'}).get('readback_verified'):raise RuntimeError('quota filler was not saved')
   staged=g.call('file_put_hex',{'name':'next.bin','hex':writer_hex})
   if not staged.get('readback_verified'):raise RuntimeError('generic writer ELF was not saved')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  g=Guest(a.qemu,guest,disk,tmp,22,out,a.transport)
  try:
   g.progress('generic writer execution',timeout_image=True,expected_exit_code=0)
   ipc=g.call('ipc_status',{})
   if ipc.get('generic_write_submitted')!=4 or ipc.get('generic_write_completed')!=4:raise RuntimeError('generic writes not completed as expected: '+str(ipc))
   if g.call('file_get',{'name':'actor.txt'})!={'name':'actor.txt','hex':b'world'.hex()}:raise RuntimeError('write did not capture initial source bytes')
   if g.call('file_get',{'name':'empty'})!={'name':'empty','hex':''}:raise RuntimeError('empty file was not created')
   reject(g,'file_get',{'name':'full'},'file not found')
   if len(g.call('file_list',{}))!=10:raise RuntimeError('rejected write changed catalog')
   if g.call('document_snapshot',{})!=final:raise RuntimeError('generic writer changed workspace')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  g=Guest(a.qemu,guest,disk,tmp,23,out,a.transport)
  try:
   # Inspect restored bytes before relying on a repeated writer execution.
   state=g.call('storage_status',{})
   if state.get('restore_generation',0)<=staged['generation']:raise RuntimeError('ELF write did not advance persistent generation')
   if g.call('file_get',{'name':'actor.txt'})!={'name':'actor.txt','hex':b'world'.hex()} or g.call('file_get',{'name':'empty'})!={'name':'empty','hex':''}:raise RuntimeError('ELF bytes did not survive VM restart')
   g.progress('generic writer after restart',timeout_image=True,expected_exit_code=0)
   reject(g,'file_get',{'name':'full'},'file not found')
   for name in ('fill','actor.txt','empty','big'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('quota fixture cleanup failed')
   # Stop the writer from re-creating files on later boot scenarios.
   if not g.call('file_put_hex',{'name':'next.bin','hex':independent_exit_elf(next_hex,0)}).get('readback_verified'):raise RuntimeError('writer reset failed')
   reject(g,'process_pair_start',{},'pair program unavailable')
   for name,value in (('a-doc','aaaa'),('b-doc','bbbb')):
    if not g.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('pair input file was not committed')
   pair_hex=[]
   for name in ('pair-a','pair-b'):
    code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/(name+'.hex')).read_text().strip())
    image=independent_elf(next_hex,code);pair_hex.append(image)
    if not g.call('file_put_hex',{'name':name+'.bin','hex':image}).get('readback_verified'):raise RuntimeError('pair ELF was not committed')
   if pair_hex[0]==pair_hex[1]:raise RuntimeError('pair programs must differ')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Restore and launch two externally stored, distinct ELF images together.
  # The kernel checks private mappings and the programs synchronize at an IRQ
  # barrier before checking different markers at their common virtual address.
  g=Guest(a.qemu,guest,disk,tmp,24,out,a.transport)
  try:
   g.progress('before two ELF admission',timeout_image=True,expected_exit_code=0)
   for i,name in enumerate(('pair-a','pair-b')):
    if g.call('file_get',{'name':name+'.bin'})!={'name':name+'.bin','hex':pair_hex[i]}:raise RuntimeError('pair ELF bytes were not restored')
   if not g.call('file_put_hex',{'name':'pair-b.bin','hex':'ff'}).get('readback_verified'):raise RuntimeError('invalid pair fixture not saved')
   reject(g,'process_pair_start',{},'invalid pair ELF')
   refused=g.call('process_pair_status',{})
   if refused.get('started') is not False or refused.get('frame_pool_assigned')!=0:raise RuntimeError('invalid pair left partial admission: '+str(refused))
   if not g.call('file_put_hex',{'name':'pair-b.bin','hex':pair_hex[1]}).get('readback_verified'):raise RuntimeError('pair fixture repair failed')
   if g.call('process_pair_start',{})!={'started':True}:raise RuntimeError('pair admission failed')
   pair=g.wait_pair()
   if pair.get('io_submitted')!=[4,4] or pair.get('io_completed')!=[4,4]:raise RuntimeError('pair file operations were not completed independently: '+str(pair))
   for name,value in (('a-doc',b'AAAA'),('b-doc',b'BBBB')):
    if g.call('file_get',{'name':name})!={'name':name,'hex':value.hex()}:raise RuntimeError('pair file value incorrect after cross-access attempts')
   if pair.get('run_id')!=1:raise RuntimeError('unexpected first pair run id')
   if g.call('document_snapshot',{})!=final:raise RuntimeError('paired tasks changed workspace')
   for name,value in (('a-doc','aaaa'),('b-doc','bbbb')):
    if not g.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('second pair inputs not reset')
   # A malformed second pair must not overwrite the finished first result.
   if not g.call('file_put_hex',{'name':'pair-b.bin','hex':'ff'}).get('readback_verified'):raise RuntimeError('second invalid pair fixture not saved')
   reject(g,'process_pair_start',{},'invalid pair ELF')
   if g.call('process_pair_status',{})!=pair:raise RuntimeError('rejected second pair changed first run state')
   if not g.call('file_put_hex',{'name':'pair-b.bin','hex':pair_hex[1]}).get('readback_verified'):raise RuntimeError('second pair fixture repair failed')
   if g.call('process_pair_start',{})!={'started':True}:raise RuntimeError('second pair admission failed')
   pair2=g.wait_pair()
   incarnations=pair2.get('incarnations',[])
   if pair2.get('run_id')!=2 or len(incarnations)!=2 or min(incarnations)<=max(pair['incarnations']):raise RuntimeError('second pair identities were reused: '+str(pair2))
   if pair2.get('stale_completion_rejected')!=[True,True] or pair2.get('old_incarnation_same_id_rejected')!=[True,True] or pair2.get('io_submitted')!=[4,4] or pair2.get('io_completed')!=[4,4]:raise RuntimeError('second pair IPC or stale completions invalid: '+str(pair2))
   for name,value in (('a-doc',b'AAAA'),('b-doc',b'BBBB')):
    if g.call('file_get',{'name':name})!={'name':name,'hex':value.hex()}:raise RuntimeError('second pair file value invalid')
   for side in ('a','b'):
    code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/('pair-cancel-'+side+'.hex')).read_text().strip())
    image=independent_elf(next_hex,code)
    if not g.call('file_put_hex',{'name':'pair-'+side+'.bin','hex':image}).get('readback_verified'):raise RuntimeError('pending-write exit fixture was not committed')
   if g.call('process_pair_start',{})!={'started':True}:raise RuntimeError('pending-write exit pair admission failed')
   exit_pair=g.wait_pair()
   if exit_pair.get('run_id')!=3 or min(exit_pair['incarnations'])<=max(pair2['incarnations']):raise RuntimeError('third pair identities were reused')
   cancelled=exit_pair.get('io_cancelled_before_apply')
   if exit_pair.get('io_submitted')!=[1,1] or exit_pair.get('io_completed')!=[0,0] or not isinstance(cancelled,list) or len(cancelled)!=2 or any(n not in (0,1) for n in cancelled):raise RuntimeError('pending-write exit accounting invalid: '+str(exit_pair))
   for i,(name,old,new) in enumerate((('a-doc',b'AAAA',b'CCCC'),('b-doc',b'BBBB',b'DDDD'))):
    expected=old if cancelled[i] else new
    if g.call('file_get',{'name':name})!={'name':name,'hex':expected.hex()}:raise RuntimeError('exit write did not match cancellation outcome')
   # Admit unrelated ELF files by explicit VFS names in this same boot.
   for name in ('pair-a.bin','pair-b.bin'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('default pair cleanup failed')
   named=[]
   for side in ('a','b'):
    code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/('pair-named-'+side+'.hex')).read_text().strip())
    named.append(independent_elf(next_hex,code))
   if not g.call('file_put_hex',{'name':'left.elf','hex':named[0]}).get('readback_verified'):raise RuntimeError('named ELF not saved')
   if not g.call('file_put_hex',{'name':'right.elf','hex':'ff'}).get('readback_verified'):raise RuntimeError('invalid named ELF not saved')
   reject(g,'process_pair_start',{'a':'left.elf'},'invalid process_pair_start arguments')
   reject(g,'process_pair_start',{'a':'left.elf','b':'absent.elf'},'pair program unavailable')
   reject(g,'process_pair_start',{'a':'left.elf','b':'right.elf'},'invalid pair ELF')
   if g.call('process_pair_status',{})!=exit_pair:raise RuntimeError('invalid named admission changed finished pair')
   if not g.call('file_put_hex',{'name':'right.elf','hex':named[1]}).get('readback_verified'):raise RuntimeError('named ELF repair failed')
   if g.call('process_pair_start',{'a':'left.elf','b':'right.elf'})!={'started':True}:raise RuntimeError('named pair admission failed')
   pair4=g.wait_pair(expected_markers=(0xc1c1,0xd2d2))
   if pair4.get('run_id')!=4 or min(pair4['incarnations'])<=max(exit_pair['incarnations']):raise RuntimeError('named pair identities were reused')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  g=Guest(a.qemu,guest,disk,tmp,25,out,a.transport)
  try:
   g.progress('pair writes after restart',timeout_image=True,expected_exit_code=0)
   reject(g,'process_pair_start',{'a':1,'b':'right.elf'},'invalid process_pair_start arguments')
   reject(g,'process_pair_start',{'a':'left.elf','b':'right.elf','extra':True},'invalid process_pair_start arguments')
   reject(g,'process_pair_start',{'a':'../left.elf','b':'right.elf'},'pair program unavailable')
   refused=g.call('process_pair_status',{})
   if refused.get('started') is not False or refused.get('frame_pool_assigned')!=0:raise RuntimeError('invalid named request allocated frames')
   # A timer can let the service apply before exit. Only the still-queued
   # requests must be cancelled; already committed writes remain committed.
   for i,(name,old,new) in enumerate((('a-doc',b'AAAA',b'CCCC'),('b-doc',b'BBBB',b'DDDD'))):
    expected=old if cancelled[i] else new
    if g.call('file_get',{'name':name})!={'name':name,'hex':expected.hex()}:raise RuntimeError('exit write did not match cancellation outcome')
   if g.call('process_pair_start',{'a':'left.elf','b':'right.elf'})!={'started':True}:raise RuntimeError('pair before single admission failed')
   before_solo_pair=g.wait_pair(expected_markers=(0xc1c1,0xd2d2))
   if not g.call('file_delete',{'name':'keep.bin'}).get('readback_verified'):raise RuntimeError('single fixture catalog preparation failed')
   reject(g,'process_start',{},'invalid process_start arguments')
   reject(g,'process_start',{'name':'absent.elf'},'program unavailable')
   if not g.call('file_put_hex',{'name':'solo.elf','hex':'ff'}).get('readback_verified'):raise RuntimeError('invalid single ELF not saved')
   before_single=g.call('process_status',{})
   reject(g,'process_start',{'name':'solo.elf'},'invalid ELF')
   if g.call('process_status',{})!=before_single:raise RuntimeError('invalid single ELF changed task state')
   code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'single.hex').read_text().strip())
   if not g.call('file_put_hex',{'name':'solo.elf','hex':independent_elf(next_hex,code)}).get('readback_verified'):raise RuntimeError('single ELF not saved')
   if g.call('process_start',{'name':'solo.elf'})!={'started':True}:raise RuntimeError('single task admission failed')
   single=g.wait_single()
   if single.get('run_id')!=before_solo_pair['run_id']+1 or single['incarnations'][0]<=max(before_solo_pair['incarnations']):raise RuntimeError('single admission did not use a fresh identity')
   if g.call('file_get',{'name':'a-doc'})!={'name':'a-doc','hex':b'SING'.hex()}:raise RuntimeError('single task file write failed')
   expected_b=b'BBBB' if cancelled[1] else b'DDDD'
   if g.call('file_get',{'name':'b-doc'})!={'name':'b-doc','hex':expected_b.hex()}:raise RuntimeError('single task changed other namespace')
   if g.call('process_pair_start',{'a':'left.elf','b':'right.elf'})!={'started':True}:raise RuntimeError('pair after single admission failed')
   after_solo_pair=g.wait_pair(expected_markers=(0xc1c1,0xd2d2))
   if after_solo_pair.get('run_id')!=single['run_id']+1 or min(after_solo_pair['incarnations'])<=single['incarnations'][0]:raise RuntimeError('pair after single reused an identity')
   for name in ('left.elf','right.elf'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('process fixture cleanup failed')
   if not g.call('file_put',{'name':'fixture.pause','data':'1'}).get('readback_verified'):raise RuntimeError('pause configuration was not saved')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  g=Guest(a.qemu,guest,disk,tmp,26,out,a.transport)
  try:
   # The exact startup file value pauses new fixtures before any ELF fixture
   # admission, while the original two built-in tasks are allowed to finish.
   for _ in range(8):
    fixture=g.call('fixtures_status',{})
    if fixture.get('admission_ready') is True:break
    time.sleep(1.5)
   else:raise RuntimeError('paused fixture state never became idle: '+str(fixture))
   if fixture.get('pause_requested') is not True or fixture.get('fixture_active') is not False or fixture.get('initial_tasks_done') is not True or fixture.get('image_count')!=0:raise RuntimeError('startup pause did not stop fixture admissions: '+str(fixture))
   if g.call('fixtures_pause',{})!=fixture:raise RuntimeError('repeated fixture pause changed idle state')
   if g.call('process_start',{'name':'solo.elf'})!={'started':True}:raise RuntimeError('external ELF during startup pause was not admitted')
   paused_single=g.wait_single()
   fixture_after=g.call('fixtures_status',{})
   if fixture_after.get('pause_requested') is not True or fixture_after.get('image_count')!=0 or fixture_after.get('fixture_active') is not False:raise RuntimeError('fixtures advanced during external task: '+str(fixture_after))
   for name in ('fixture.pause','solo.elf'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('startup pause configuration cleanup failed')
   resumed=g.call('fixtures_resume',{})
   if resumed.get('pause_requested') is not False:raise RuntimeError('fixture resume did not clear pause')
   g.progress('fixtures after external task and resume',timeout_image=True,expected_exit_code=0)
   for i,(name,old,new) in enumerate((('a-doc',b'SING',b'SING'),('b-doc',b'BBBB',b'DDDD'))):
    expected=old if cancelled[i] else new
    if g.call('file_get',{'name':name})!={'name':name,'hex':expected.hex()}:raise RuntimeError('pending-write outcome did not survive restart')
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('pair data cleanup failed')
   # Independent ELFs wait on separate VFS gates. Stop A by exact token,
   # readmit its slot, reject the old token and only then release the gates.
   for side in ('a','b'):
    if not g.call('file_put',{'name':side+'-gate','data':'0'}).get('readback_verified'):raise RuntimeError('independent gate not saved')
    code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/('independent-'+side+'.hex')).read_text().strip())
    if not g.call('file_put_hex',{'name':side+'-ind.elf','hex':independent_elf(next_hex,code)}).get('readback_verified'):raise RuntimeError('independent ELF not saved')
   if not g.call('file_put',{'name':'fixture.pause','data':'1'}).get('readback_verified'):raise RuntimeError('independent pause configuration not saved')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  g=Guest(a.qemu,guest,disk,tmp,27,out,a.transport)
  try:
   for _ in range(8):
    fixture=g.call('fixtures_status',{})
    if fixture.get('admission_ready') is True:break
    time.sleep(1.5)
   else:raise RuntimeError('independent startup pause did not become idle: '+str(fixture))
   if fixture.get('pause_requested') is not True or fixture.get('image_count')!=0:raise RuntimeError('independent scenario did not pause fixtures')
   g.wait_boot_identity()
   task_a=g.call('process_spawn',{'name':'a-ind.elf'})
   if task_a.get('started') is not True or task_a.get('slot')!=0 or not isinstance(task_a.get('incarnation'),int) or task_a.get('incarnation_text')!=str(task_a['incarnation']) or len(task_a.get('boot_id',''))!=32:raise RuntimeError('first independent admission failed: '+str(task_a))
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='ready' and s.get('io_completed',[0,0])[0]>0,'A live and reading')
   if state.get('incarnations',[None,None])[0]!=task_a['incarnation']:raise RuntimeError('A identity changed before B admission')
   b=g.call('process_spawn',{'name':'b-ind.elf'})
   if b.get('started') is not True or b.get('slot')!=1 or not isinstance(b.get('incarnation'),int) or b['incarnation']<=task_a['incarnation'] or b.get('boot_id')!=task_a['boot_id']:raise RuntimeError('second independent admission failed: '+str(b))
   state=g.wait_independent(lambda s:s.get('slot_states')==['ready','ready'] and min(s.get('io_completed',[0,0]))>0,'A and B live and reading')
   if state.get('incarnations')!=[task_a['incarnation'],b['incarnation']] or state.get('frame_pool_assigned')!=4:raise RuntimeError('independent tasks not isolated concurrently: '+str(state))
   b_reads=state['io_completed'][1]
   reject(g,'process_stop',{'slot':0,'incarnation':task_a['incarnation'],'boot_id':task_a['boot_id']},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':0,'incarnation':'0'+task_a['incarnation_text'],'boot_id':task_a['boot_id']},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':0,'incarnation':'18446744073709551616','boot_id':task_a['boot_id']},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':0,'incarnation':task_a['incarnation_text'],'boot_id':'G'+task_a['boot_id'][1:]},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':3,'incarnation':task_a['incarnation_text'],'boot_id':task_a['boot_id']},'invalid process slot')
   stopped=g.call('process_stop',{'slot':0,'incarnation':task_a['incarnation_text'],'boot_id':task_a['boot_id']})
   if stopped!={'stopped':True,'slot':0,'incarnation':task_a['incarnation_text'],'boot_id':task_a['boot_id']}:raise RuntimeError('targeted stop failed: '+str(stopped))
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','ready'] and s.get('io_completed',[0,0])[1]>b_reads,'A exited while B continued reading')
   if state.get('incarnations')!=[task_a['incarnation'],b['incarnation']] or state.get('outcomes',[None,None])[0]!=4 or state.get('exit_codes',[0,0])[0] is not None or state.get('data_scrubbed',[False,False])[0] is not True or state.get('code_scrubbed',[False,False])[0] is not True or state.get('frame_pool_assigned')!=2:raise RuntimeError('A teardown disturbed B: '+str(state))
   b_reads=state['io_completed'][1]
   if not g.call('file_put',{'name':'a-gate','data':'0'}).get('readback_verified'):raise RuntimeError('A gate reset failed')
   a2=g.call('process_spawn',{'name':'a-ind.elf'})
   if a2.get('started') is not True or a2.get('slot')!=0 or not isinstance(a2.get('incarnation'),int) or a2.get('incarnation_text')!=str(a2['incarnation']) or a2['incarnation']<=b['incarnation'] or a2.get('boot_id')!=task_a['boot_id']:raise RuntimeError('A readmission invalid: '+str(a2))
   reject(g,'process_stop',{'slot':0,'incarnation':task_a['incarnation_text'],'boot_id':task_a['boot_id']},'stale process incarnation')
   state=g.wait_independent(lambda s:s.get('slot_states')==['ready','ready'] and s.get('io_completed',[0,0])[0]>0 and s.get('io_completed',[0,0])[1]>b_reads,'A readmitted while B continued reading')
   if state.get('incarnations')!=[a2['incarnation'],b['incarnation']] or state.get('frame_pool_assigned')!=4:raise RuntimeError('A readmission disturbed B: '+str(state))
   if not g.call('file_put',{'name':'a-gate','data':'1'}).get('readback_verified'):raise RuntimeError('A second release failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','ready'],'readmitted A exited while B live')
   if state.get('incarnations')!=[a2['incarnation'],b['incarnation']] or state.get('exit_codes',[None,None])[0]!=0 or state.get('frame_pool_assigned')!=2:raise RuntimeError('second A teardown disturbed B: '+str(state))
   if not g.call('file_put',{'name':'b-gate','data':'1'}).get('readback_verified'):raise RuntimeError('B gate release failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'both tasks exited')
   if state.get('exit_codes')!=[0,0] or state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('independent task cleanup failed: '+str(state))
   if not g.call('file_put',{'name':'a-gate','data':'0'}).get('readback_verified'):raise RuntimeError('cross-boot gate reset failed')
   for name in ('b-gate','b-ind.elf'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('independent fixture cleanup failed')
   if g.call('fixtures_resume',{}).get('pause_requested') is not False:raise RuntimeError('independent fixture resume failed')
   after=g.progress('fixtures resumed after independent processes',timeout_image=True,expected_exit_code=0)
   if after.get('private_values')!=[0xa1a1,0xb2b2]:raise RuntimeError('independent mode changed historical fixture status')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Repeat the same slot/incarnation after a complete boot on the same disk.
  # Only the fresh boot ID distinguishes it from the earlier stop request.
  g=Guest(a.qemu,guest,disk,tmp,28,out,a.transport)
  try:
   for _ in range(8):
    fixture=g.call('fixtures_status',{})
    if fixture.get('admission_ready') is True:break
    time.sleep(1.5)
   else:raise RuntimeError('replay boot did not reach paused admission')
   g.wait_boot_identity()
   replay=g.call('process_spawn',{'name':'a-ind.elf'})
   if replay.get('slot')!=task_a['slot'] or replay.get('incarnation_text')!=task_a['incarnation_text'] or replay.get('boot_id')==task_a['boot_id']:raise RuntimeError('cross-boot replay did not isolate the boot identity: '+str(replay))
   reject(g,'process_stop',{'slot':task_a['slot'],'incarnation':task_a['incarnation_text'],'boot_id':task_a['boot_id']},'stale boot identity')
   live=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='ready' and s.get('io_completed',[0,0])[0]>0,'new boot task survived old stop')
   if live.get('incarnations',[None,None])[0]!=replay['incarnation']:raise RuntimeError('stale boot stop changed the new task')
   stopped=g.call('process_stop',{'slot':replay['slot'],'incarnation':replay['incarnation_text'],'boot_id':replay['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('current boot stop failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'new boot task stopped')
   if state.get('outcomes',[None,None])[0]!=4 or state.get('frame_pool_assigned')!=0:raise RuntimeError('new boot stop did not reclaim resources')
   for name in ('fixture.pause','a-gate'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('replay fixture cleanup failed')
   if g.call('fixtures_resume',{}).get('pause_requested') is not False:raise RuntimeError('replay fixture resume failed')
   g.progress('replay boot fixture resume',timeout_image=True,expected_exit_code=0)
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Boot without a VirtIO RNG: management remains available, but creating or
  # stopping independent tasks cannot use an invented boot identity.
  g=Guest(a.qemu,guest,disk,tmp,29,out,a.transport,rng=False)
  try:
   g.progress('boot without entropy',timeout_image=True,expected_exit_code=0)
   if g.call('process_status',{}).get('boot_identity_ready') is not False:raise RuntimeError('boot identity available without entropy device')
   reject(g,'process_spawn',{'name':'a-ind.elf'},'boot identity unavailable')
   reject(g,'process_stop',{'slot':task_a['slot'],'incarnation':task_a['incarnation_text'],'boot_id':task_a['boot_id']},'boot identity unavailable')
   if g.call('file_get',{'name':'note'})!={'name':'note','hex':b'hello'.hex()}:raise RuntimeError('service unavailable without entropy')
   if not g.call('file_delete',{'name':'a-ind.elf'}).get('readback_verified'):raise RuntimeError('no-entropy fixture cleanup failed')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Upload a real compiled Rust no_std program larger than the persistent
  # per-file limit; the staging area is not part of the virtual disk catalog.
  # Guard sectors outside the designated program region in the dedicated disk.
  guarded=bytearray(disk.read_bytes());guarded[3*512]=0xa7;guarded[40*512]=0x5c;guarded[63*512]=0x6d;guarded[130*512]=0x7e;disk.write_bytes(guarded)
  rust_code=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-no-std.hex').read_text().strip())
  if not 392<len(rust_code)<=4096 or len(rust_code)%4:raise RuntimeError('Rust code size not suitable for page-boundary test')
  # Pad unreachable code with AArch64 NOPs to exercise the full RX page.
  rust_code+=bytes.fromhex('1f2003d5')*((4096-len(rust_code))//4)
  rust_elf=bytes.fromhex(independent_elf(next_hex,rust_code))
  if len(rust_elf)!=4216:raise RuntimeError('Rust ELF does not fill the maximum code page')
  g=Guest(a.qemu,guest,disk,tmp,30,out,a.transport)
  try:
   g.progress('before staged Rust ELF',timeout_image=True,expected_exit_code=0)
   reject(g,'upload_begin',{'size':16385,'checksum':'0000000000000000'},'upload size outside bound')
   # A coherent legacy RX segment larger than one page enters staging but
   # must fail loader validation before it can be admitted.
   too_large=bytearray(rust_elf)+bytes.fromhex('1f2003d5')
   too_large[96:104]=(4100).to_bytes(8,'little');too_large[104:112]=(4100).to_bytes(8,'little')
   too_large_id=g.call('upload_begin',{'size':len(too_large),'checksum':fnv64(too_large)})['id']
   for offset in range(0,len(too_large),512):g.call('upload_chunk',{'id':too_large_id,'offset':offset,'hex':too_large[offset:offset+512].hex()})
   reject(g,'upload_seal',{'id':too_large_id},'invalid staged ELF')
   g.call('upload_reset',{'id':too_large_id})
   checksum=fnv64(rust_elf)
   damaged=bytearray(rust_elf);damaged[-1]^=1
   bad_id=g.call('upload_begin',{'size':len(damaged),'checksum':checksum})['id']
   for offset in range(0,len(damaged),512):
    if g.call('upload_chunk',{'id':bad_id,'offset':offset,'hex':damaged[offset:offset+512].hex()})!={'received':min(offset+512,len(damaged))}:raise RuntimeError('damaged upload chunk accounting invalid')
   reject(g,'upload_seal',{'id':bad_id},'upload checksum mismatch')
   if g.call('upload_reset',{'id':bad_id})!={'reset':True}:raise RuntimeError('damaged upload reset failed')
   invalid=bytearray.fromhex(independent_elf(next_hex,bytes.fromhex('1f2003d5')))
   invalid[104:112]=(4097).to_bytes(8,'little') # impossible > one-page segment
   invalid_id=g.call('upload_begin',{'size':len(invalid),'checksum':fnv64(invalid)})['id']
   reject(g,'upload_chunk',{'id':invalid_id,'offset':0,'hex':'00'*(len(invalid)+1)},'upload exceeds declared size')
   reject(g,'upload_chunk',{'id':invalid_id,'offset':0,'hex':''},'upload chunk outside bound')
   g.call('upload_chunk',{'id':invalid_id,'offset':0,'hex':invalid.hex()})
   reject(g,'upload_seal',{'id':invalid_id},'invalid staged ELF')
   g.call('upload_reset',{'id':invalid_id})
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Start a fresh guest for the valid upload to exercise the fact that staging
  # is intentionally volatile and does not survive a restart.
  g=Guest(a.qemu,guest,disk,tmp,32,out,a.transport)
  try:
   g.progress('before valid staged Rust ELF',timeout_image=True,expected_exit_code=0)
   g.wait_boot_identity()
   original_catalog=g.call('file_list',{})
   upload_id=g.call('upload_begin',{'size':len(rust_elf),'checksum':checksum})['id']
   reject(g,'upload_seal',{'id':upload_id},'upload incomplete')
   reject(g,'process_spawn_staged',{'id':upload_id},'upload not sealed')
   reject(g,'upload_chunk',{'id':invalid_id,'offset':0,'hex':'00'},'stale upload id')
   reject(g,'upload_chunk',{'id':upload_id,'offset':1,'hex':'00'},'upload offset mismatch')
   reject(g,'upload_chunk',{'id':upload_id,'offset':0,'hex':'00'*513},'file too large')
   for offset in range(0,len(rust_elf),512):
    if g.call('upload_chunk',{'id':upload_id,'offset':offset,'hex':rust_elf[offset:offset+512].hex()})!={'received':min(offset+512,len(rust_elf))}:raise RuntimeError('Rust upload chunk accounting invalid')
    if offset==0:reject(g,'upload_chunk',{'id':upload_id,'offset':0,'hex':'00'},'upload offset mismatch')
   if g.call('upload_seal',{'id':upload_id})!={'sealed':True}:raise RuntimeError('Rust upload seal failed')
   reject(g,'upload_chunk',{'id':upload_id,'offset':len(rust_elf),'hex':'00'},'upload already sealed')
   if g.call('file_list',{})!=original_catalog:raise RuntimeError('temporary upload modified VFS')
   if g.call('program_store_status',{})!={'state':'blank','slots':['blank','blank'],'generation':None,'write_blocked':False,'unknown_region':False}:raise RuntimeError('new program area not blank')
   disk_before=disk.read_bytes()
   for index in (0,1):
    if g.call('program_store_commit',{'id':upload_id})!={'slot':index,'generation':index+1,'readback_verified':True}:raise RuntimeError('program store commit failed')
   disk_after=disk.read_bytes()
   if disk_before[:20*512]!=disk_after[:20*512] or disk_before[40*512:]!=disk_after[40*512:]:raise RuntimeError('program store changed sectors outside its area')
   rust=g.call('process_spawn_staged',{'id':upload_id})
   if rust.get('started') is not True or rust.get('slot')!=0:raise RuntimeError('staged Rust admission failed')
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','staged Rust ELF exited')
   if state.get('incarnations',[None,None])[0]!=rust.get('incarnation') or state.get('outcomes',[None,None])[0]!=1 or state.get('exit_codes',[None,None])[0]!=0 or state.get('data_scrubbed',[False,False])[0] is not True or state.get('code_scrubbed',[False,False])[0] is not True or state.get('frame_pool_assigned')!=0:raise RuntimeError('staged Rust ELF exit or cleanup invalid: '+str(state))
   if g.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('Rust program did not perform its file write')
   if g.call('upload_status',{}).get('sealed') is not True:raise RuntimeError('sealed Rust upload vanished before reboot')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  g=Guest(a.qemu,guest,disk,tmp,31,out,a.transport)
  try:
   g.progress('after staged Rust ELF reboot',timeout_image=True,expected_exit_code=0)
   if g.call('upload_status',{}).get('active') is not False:raise RuntimeError('temporary upload survived reboot')
   if g.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('Rust file effect not persisted')
   stored=g.call('program_store_status',{})
   if stored!={'state':'valid','slots':['valid','valid'],'generation':2,'write_blocked':False,'unknown_region':False}:raise RuntimeError('stored Rust ELF not restored: '+str(stored))
   if not g.call('file_put',{'name':'a-rust','data':'X'}).get('readback_verified'):raise RuntimeError('stored ELF execution probe setup failed')
   g.wait_boot_identity()
   admitted=g.call('program_store_spawn',{})
   if admitted.get('started') is not True or admitted.get('slot')!=0:raise RuntimeError('stored ELF admission failed')
   wait_rust_program(g,admitted)
   if g.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('restored ELF did not execute its file write')
   if not g.call('file_delete',{'name':'a-rust'}).get('readback_verified'):raise RuntimeError('Rust fixture cleanup failed')
   heap_samples=exercise_reclaiming_heap(g,preflight=a.heap_preflight)
   if not g.call('file_put',{'name':'a-heap','data':'alive'}).get('readback_verified'):raise RuntimeError('file write after heap workload failed')
   if g.call('file_get',{'name':'a-heap'})!={'name':'a-heap','hex':b'alive'.hex()}:raise RuntimeError('file read after heap workload failed')
   if not g.call('file_delete',{'name':'a-heap'}).get('readback_verified'):raise RuntimeError('file cleanup after heap workload failed')
   g.wait_boot_identity()
   launched=g.call('process_spawn',{'name':'next.bin'})
   if launched.get('started') is not True or launched.get('slot')!=0:raise RuntimeError('process admission after heap workload failed')
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','post-workload ELF exited')
   if state.get('incarnations',[None,None])[0]!=launched.get('incarnation') or state.get('outcomes',[None,None])[0]!=1 or state.get('exit_codes',[None,None])[0]!=0 or state.get('data_scrubbed',[False,False])[0] is not True or state.get('code_scrubbed',[False,False])[0] is not True or state.get('frame_pool_assigned')!=0:raise RuntimeError('process cleanup after heap workload failed')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Exercise conservative recovery on disposable clones of the same disk.
  # The source disk and its two snapshot slots are never overwritten by the
  # host while a guest is running.
  program_base=disk.read_bytes()
  def program_case(index,raw,expected_slots,expected_state,load_error,commit_error):
   case_disk=Path(tmp)/('program-case-'+str(index)+'.raw');create(case_disk,bytes(raw))
   case=Guest(a.qemu,guest,case_disk,tmp,index,out,a.transport)
   try:
    case.progress('program region case '+str(index),timeout_image=True,expected_exit_code=0)
    status=case.call('program_store_status',{})
    if status.get('slots')!=expected_slots or status.get('state')!=expected_state or status.get('unknown_region')!=('unknown' in expected_slots):raise RuntimeError('wrong program recovery status: '+str(status))
    if load_error is None:
     case.wait_boot_identity();admitted=case.call('program_store_spawn',{});wait_rust_program(case,admitted)
     if case.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('program fallback did not execute')
    else:reject(case,'program_store_spawn',{},load_error)
    upload_id=stage_program(case,rust_elf)
    before=case_disk.read_bytes()
    reject(case,'program_store_commit',{'id':upload_id},commit_error)
    if case_disk.read_bytes()!=before:raise RuntimeError('refused program write changed the virtual disk')
    traces.append(case.trace);states.append(case.call('storage_status',{}))
   finally:case.close()
  unknown=bytearray(program_base);unknown[30*512:30*512+8]=b'FOREIGN!'
  program_case(33,unknown,['valid','unknown'],'valid',None,'unknown program region')
  corrupt=bytearray(program_base);corrupt[30*512+512+200]^=1
  program_case(34,corrupt,['valid','invalid'],'valid',None,'program region needs repair')
  invalid=bytearray(program_base);invalid[20*512+512+200]^=1;invalid[30*512+512+200]^=1
  program_case(35,invalid,['invalid','invalid'],'invalid','no valid committed program','program region needs repair')
  partial=bytearray(program_base);partial[20*512+28]=0;partial[30*512:40*512]=bytes(5120)
  program_case(36,partial,['invalid','blank'],'invalid','no valid committed program','program region needs repair')
  unknown_first=bytearray(program_base);unknown_first[20*512:20*512+8]=b'FOREIGN!'
  program_case(37,unknown_first,['unknown','valid'],'valid',None,'unknown program region')
  duplicate=bytearray(program_base);record=bytearray(duplicate[30*512:40*512]);record[8:16]=(1).to_bytes(8,'little')
  payload_len=int.from_bytes(record[16:20],'little');record[20:28]=int(fnv64(record[8:20]+record[512:512+payload_len]),16).to_bytes(8,'little');duplicate[30*512:40*512]=record
  program_case(38,duplicate,['valid','valid'],'invalid','duplicate program generations','duplicate program generations')
  # Real linked Rust executable with RX, RW/BSS and a separate guarded stack.
  linked=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-two-segment.hex').read_text().strip())
  if not 4216<len(linked)<=16384:raise RuntimeError('linked Rust ELF does not exercise extended staging')
  g=Guest(a.qemu,guest,disk,tmp,41,out,a.transport)
  try:
   g.progress('before linked Rust tasks',timeout_image=True,expected_exit_code=0);g.wait_boot_identity()
   compact=compact_two_segment(linked)
   # A compact repack of the genuine linked segments is accepted first;
   # each subsequent image changes one structural property at a time.
   baseline=stage_program(g,compact);g.call('upload_reset',{'id':baseline})
   malformed=[]
   bad=bytearray(compact);struct.pack_into('<I',bad,124,7);malformed.append(bad) # RWX
   bad=bytearray(compact);struct.pack_into('<Q',bad,160,4);malformed.append(bad) # filesz > memsz
   bad=bytearray(compact);struct.pack_into('<Q',bad,160,4097);malformed.append(bad) # RW beyond page
   bad=bytearray(compact);struct.pack_into('<Q',bad,128,struct.unpack_from('<Q',bad,72)[0]);malformed.append(bad) # file overlap
   bad=bytearray(compact);struct.pack_into('<Q',bad,24,0x50000000);malformed.append(bad) # entry outside RX
   bad=bytearray(compact);struct.pack_into('<Q',bad,168,4096);malformed.append(bad) # bad p_align congruence
   bad=bytearray(compact);struct.pack_into('<I',bad,120,3);malformed.append(bad) # PT_INTERP
   for image in malformed:
    invalid_id=g.call('upload_begin',{'size':len(image),'checksum':fnv64(image)})['id']
    for offset in range(0,len(image),512):g.call('upload_chunk',{'id':invalid_id,'offset':offset,'hex':image[offset:offset+512].hex()})
    reject(g,'upload_seal',{'id':invalid_id},'invalid staged ELF')
    g.call('upload_reset',{'id':invalid_id})
   for side in ('a','b'):
    if not g.call('file_put',{'name':side+'-gate','data':'0'}).get('readback_verified'):raise RuntimeError('linked Rust gate not saved')
   upload_id=stage_program(g,linked)
   before=disk.read_bytes();reject(g,'program_store_commit',{'id':upload_id},'invalid staged ELF')
   if disk.read_bytes()!=before:raise RuntimeError('oversized persistent program modified disk')
   if g.call('program_store_v2_status',{})!={'state':'blank','slots':['blank','blank'],'generation':None,'write_blocked':False,'unknown_region':False}:raise RuntimeError('new V2 program area not blank')
   if g.call('program_store_v2_commit',{'id':upload_id})!={'slot':0,'generation':1,'readback_verified':True}:raise RuntimeError('linked V2 commit failed')
   after=disk.read_bytes()
   if before[:64*512]!=after[:64*512] or before[130*512:]!=after[130*512:]:raise RuntimeError('V2 commit changed sectors outside its area')
   task_a=g.call('process_spawn_staged',{'id':upload_id})
   if task_a.get('slot')!=0:raise RuntimeError('linked Rust A admission failed')
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='ready' and s.get('io_completed',[0,0])[0]>=11,'linked Rust A active after buffer checks')
   task_b=g.call('process_spawn_staged',{'id':upload_id})
   if task_b.get('slot')!=1 or task_b.get('incarnation',0)<=task_a.get('incarnation',0):raise RuntimeError('linked Rust B admission failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['ready','ready'] and min(s.get('io_completed',[0,0]))>=11,'both linked Rust tasks active after buffer checks')
   if state.get('incarnations')!=[task_a['incarnation'],task_b['incarnation']] or state.get('two_segment')!=[True,True] or state.get('rw_init_verified')!=[True,True] or state.get('slot_mappings_verified')!=[True,True] or state.get('frame_pool_assigned')!=6:raise RuntimeError('linked Rust mappings or initialization invalid: '+str(state))
   for side in ('a','b'):
    if g.call('file_get',{'name':side+'-rust'})!={'name':side+'-rust','hex':'52'}:raise RuntimeError('linked Rust task did not write its file')
   b_reads=state['io_completed'][1]
   if not g.call('file_put',{'name':'a-gate','data':'1'}).get('readback_verified'):raise RuntimeError('linked Rust A release failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','ready'] and s.get('io_completed',[0,0])[1]>b_reads,'linked Rust B active after A exit')
   if state.get('incarnations')!=[task_a['incarnation'],task_b['incarnation']] or state.get('exit_codes',[None,None])[0]!=0 or state.get('stack_scrubbed')!=[True,False] or state.get('frame_pool_assigned')!=3:raise RuntimeError('linked Rust A cleanup disturbed B: '+str(state))
   if not g.call('file_put',{'name':'b-gate','data':'1'}).get('readback_verified'):raise RuntimeError('linked Rust B release failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'linked Rust tasks exited')
   if state.get('outcomes')!=[1,1] or state.get('exit_codes')!=[0,0] or state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('stack_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('linked Rust teardown invalid: '+str(state))
   g.call('upload_reset',{'id':upload_id})
   linked_max=linked+bytes(16384-len(linked))
   upload_max=stage_program(g,linked_max)
   before=disk.read_bytes()
   if g.call('program_store_v2_commit',{'id':upload_max})!={'slot':1,'generation':2,'readback_verified':True}:raise RuntimeError('maximum V2 commit failed')
   after=disk.read_bytes()
   if before[:64*512]!=after[:64*512] or before[130*512:]!=after[130*512:]:raise RuntimeError('maximum V2 commit changed outside its area')
   g.call('upload_reset',{'id':upload_max})
   for side in ('a','b'):
    for suffix in ('-gate','-rust'):
     if not g.call('file_delete',{'name':side+suffix}).get('readback_verified'):raise RuntimeError('linked Rust fixture cleanup failed')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Restore and run the full-sized V2 record after a fresh VM boot.
  g=Guest(a.qemu,guest,disk,tmp,42,out,a.transport)
  try:
   g.progress('before persistent V2 Rust',timeout_image=True,expected_exit_code=0);g.wait_boot_identity()
   v2=g.call('program_store_v2_status',{})
   if v2!={'state':'valid','slots':['valid','valid'],'generation':2,'write_blocked':False,'unknown_region':False}:raise RuntimeError('V2 program did not restore: '+str(v2))
   if g.call('program_store_status',{}).get('generation')!=2:raise RuntimeError('V2 changed V1 program generation')
   if not g.call('file_put',{'name':'a-gate','data':'1'}).get('readback_verified'):raise RuntimeError('V2 Rust gate setup failed')
   task=g.call('program_store_v2_spawn',{})
   if task.get('started') is not True or task.get('slot')!=0:raise RuntimeError('V2 persistent ELF admission failed')
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','stored V2 Rust exited')
   if state.get('outcomes',[None,None])[0]!=1 or state.get('exit_codes',[None,None])[0]!=0 or state.get('two_segment',[False,False])[0] is not True or state.get('rw_init_verified',[False,False])[0] is not True or state.get('stack_scrubbed',[False,False])[0] is not True or state.get('frame_pool_assigned')!=0:raise RuntimeError('V2 persistent ELF execution failed: '+str(state))
   if g.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('V2 persistent ELF file effect absent')
   for name in ('a-gate','a-rust'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('V2 ELF fixture cleanup failed')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  v2_base=disk.read_bytes()
  def v2_case(index,raw,expected_slots,expected_state,load_error,commit_error):
   case_disk=Path(tmp)/('program-v2-case-'+str(index)+'.raw');create(case_disk,bytes(raw))
   case=Guest(a.qemu,guest,case_disk,tmp,index,out,a.transport)
   try:
    case.progress('program V2 region case '+str(index),timeout_image=True,expected_exit_code=0)
    status=case.call('program_store_v2_status',{})
    if status.get('slots')!=expected_slots or status.get('state')!=expected_state or status.get('unknown_region')!=('unknown' in expected_slots):raise RuntimeError('wrong V2 recovery status: '+str(status))
    if load_error is None:
     if not case.call('file_put',{'name':'a-gate','data':'1'}).get('readback_verified'):raise RuntimeError('V2 fallback gate failed')
     case.wait_boot_identity();task=case.call('program_store_v2_spawn',{})
     state=case.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','V2 fallback exited')
     if state.get('incarnations',[None,None])[0]!=task.get('incarnation') or state.get('exit_codes',[None,None])[0]!=0 or state.get('stack_scrubbed',[False,False])[0] is not True:raise RuntimeError('V2 fallback failed: '+str(state))
     if case.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('V2 fallback did not execute')
    else:reject(case,'program_store_v2_spawn',{},load_error)
    upload_id=stage_program(case,linked)
    before=case_disk.read_bytes();reject(case,'program_store_v2_commit',{'id':upload_id},commit_error)
    if case_disk.read_bytes()!=before:raise RuntimeError('refused V2 write changed virtual disk')
    traces.append(case.trace);states.append(case.call('storage_status',{}))
   finally:case.close()
  v2_unknown=bytearray(v2_base);v2_unknown[97*512:97*512+8]=b'FOREIGN!'
  v2_case(43,v2_unknown,['valid','unknown'],'valid',None,'unknown program region')
  v2_unknown_first=bytearray(v2_base);v2_unknown_first[64*512:64*512+8]=b'FOREIGN!'
  v2_case(44,v2_unknown_first,['unknown','valid'],'valid',None,'unknown program region')
  v2_corrupt=bytearray(v2_base);v2_corrupt[97*512+512+200]^=1
  v2_case(45,v2_corrupt,['valid','invalid'],'valid',None,'program region needs repair')
  v2_invalid=bytearray(v2_base);v2_invalid[64*512+512+200]^=1;v2_invalid[97*512+512+200]^=1
  v2_case(46,v2_invalid,['invalid','invalid'],'invalid','no valid committed program','program region needs repair')
  v2_partial=bytearray(v2_base);v2_partial[64*512+28]=0;v2_partial[97*512:130*512]=bytes(16896)
  v2_case(47,v2_partial,['invalid','blank'],'invalid','no valid committed program','program region needs repair')
  v2_duplicate=bytearray(v2_base);record=bytearray(v2_duplicate[97*512:130*512]);record[8:16]=(1).to_bytes(8,'little')
  payload_len=int.from_bytes(record[16:20],'little');record[20:28]=int(fnv64(record[8:20]+record[512:512+payload_len]),16).to_bytes(8,'little');v2_duplicate[97*512:130*512]=record
  v2_case(48,v2_duplicate,['valid','valid'],'invalid','duplicate program generations','duplicate program generations')
  # A linked Rust program exchanges messages directly between two private
  # address spaces. Rebind after recycling A while B remains alive.
  ipc_elf=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-direct-ipc.hex').read_text().strip())
  g=Guest(a.qemu,guest,disk,tmp,51,out,a.transport)
  try:
   g.progress('before direct IPC tasks',timeout_image=True,expected_exit_code=0);g.wait_boot_identity()
   files_before=g.call('file_list',{})
   upload_id=stage_program(g,ipc_elf)
   first=g.call('process_spawn_staged',{'id':upload_id})
   second=g.call('process_spawn_staged',{'id':upload_id})
   if first.get('slot')!=0 or second.get('slot')!=1 or first.get('boot_id')!=second.get('boot_id') or first.get('incarnation')==second.get('incarnation'):raise RuntimeError('direct IPC admissions invalid')
   state=g.wait_independent(lambda s:s.get('slot_states')==['ready','ready'],'direct IPC tasks ready')
   if state.get('two_segment')!=[True,True] or state.get('frame_pool_assigned')!=6:raise RuntimeError('direct IPC task isolation invalid: '+str(state))
   args={'a_slot':0,'a_incarnation':first['incarnation_text'],'b_slot':1,'b_incarnation':second['incarnation_text'],'boot_id':first['boot_id']}
   reject(g,'ipc_direct_bind',dict(args,b_slot=0),'invalid channel slots')
   if g.call('ipc_direct_bind',args).get('bound') is not True:raise RuntimeError('direct IPC bind failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','ready'],'first IPC exchange and A exit')
   if state.get('exit_codes',[None,None])[0]!=0 or state.get('outcomes',[None,None])[0]!=1 or state.get('incarnations')!=[first['incarnation'],second['incarnation']] or state.get('frame_pool_assigned')!=3:raise RuntimeError('first IPC exchange failed: '+str(state))
   channel=g.call('ipc_direct_status',{})
   if set(channel)!={'channels','bound','channel_id','slots','incarnations','queued','sent','received','dropped','scrubbed','full_rejections','peer_unbound','all_peer_unbound','waiting','wait_count','wake_message','wake_close','wake_timeout','all_waiting','all_wait_count','all_wake_message','all_wake_close','all_wake_timeout'} or channel.get('bound') is not False or channel.get('queued')!=[0,0] or channel.get('sent')!=[2,2] or channel.get('received')!=[2,2] or channel.get('scrubbed') is not True:raise RuntimeError('direct IPC closure or first exchange invalid: '+str(channel))
   replacement=g.call('process_spawn_staged',{'id':upload_id})
   if replacement.get('slot')!=0 or replacement.get('incarnation',0)<=second['incarnation'] or replacement.get('boot_id')!=first['boot_id']:raise RuntimeError('IPC sender readmission invalid')
   reject(g,'ipc_direct_bind',args,'stale process incarnation')
   args['a_incarnation']=replacement['incarnation_text']
   if g.call('ipc_direct_bind',args).get('bound') is not True:raise RuntimeError('direct IPC rebind failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'second IPC exchange and teardown')
   if state.get('incarnations')!=[replacement['incarnation'],second['incarnation']] or state.get('outcomes')!=[1,1] or state.get('exit_codes')!=[0,0] or state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('stack_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('direct IPC process cleanup invalid: '+str(state))
   channel=g.call('ipc_direct_status',{})
   if channel.get('bound') is not False or channel.get('queued')!=[0,0] or channel.get('sent')!=[2,2] or channel.get('received')!=[2,2] or channel.get('scrubbed') is not True:raise RuntimeError('direct IPC final cleanup invalid: '+str(channel))
   if g.call('file_list',{})!=files_before:raise RuntimeError('direct IPC unexpectedly modified the file catalogue')
   g.call('upload_reset',{'id':upload_id})
   # Exercise an actually full FIFO and explicit close/reopen on the same boot.
   queue_elf=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-ipc-queue.hex').read_text().strip())
   queue_upload=stage_program(g,queue_elf)
   qa=g.call('process_spawn_staged',{'id':queue_upload})
   qb=g.call('process_spawn_staged',{'id':queue_upload})
   if qa.get('slot')!=0 or qb.get('slot')!=1 or qa.get('boot_id')!=qb.get('boot_id'):raise RuntimeError('IPC queue fixture admission failed')
   g.wait_independent(lambda x:x.get('slot_states')==['ready','ready'],'IPC queue actors ready')
   queue_args={'a_slot':0,'a_incarnation':qa['incarnation_text'],'b_slot':1,'b_incarnation':qb['incarnation_text'],'boot_id':qa['boot_id']}
   first_channel=g.call('ipc_direct_bind',queue_args)['channel_id']
   def wait_direct(predicate,label):
    for _ in range(60):
     snapshot=g.call('ipc_direct_status',{})
     if predicate(snapshot):return snapshot
     time.sleep(.2)
    raise RuntimeError('direct IPC did not reach '+label+': '+str(snapshot))
   full=wait_direct(lambda x:x.get('queued')==[0,2] and x.get('full_rejections',[0,0])[0]>0,'full queue and rejected send')
   baseline=full['peer_unbound']
   if g.call('ipc_direct_close',{'channel_id':first_channel,'boot_id':qa['boot_id']})!={'closed':True}:raise RuntimeError('IPC close failed')
   closed=wait_direct(lambda x:x.get('bound') is False and all(x.get('peer_unbound',[0,0])[i]>baseline[i] for i in range(2)),'both tasks observed close')
   if closed.get('queued')!=[0,0] or closed.get('dropped')!=[0,2] or closed.get('scrubbed') is not True:raise RuntimeError('IPC close did not scrub full queue: '+str(closed))
   second_channel=g.call('ipc_direct_bind',queue_args)['channel_id']
   if int(second_channel)<=int(first_channel):raise RuntimeError('IPC channel identity did not advance')
   wait_direct(lambda x:x.get('bound') is True and x.get('received',[0,0])[0]==1,'empty new queue and new message')
   reject(g,'ipc_direct_close',{'channel_id':first_channel,'boot_id':qa['boot_id']},'stale direct channel')
   if g.call('ipc_direct_close',{'channel_id':second_channel,'boot_id':qa['boot_id']})!={'closed':True}:raise RuntimeError('second IPC close failed')
   state=g.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'IPC queue actors exited')
   if state.get('incarnations')!=[qa['incarnation'],qb['incarnation']] or state.get('outcomes')!=[1,1] or state.get('exit_codes')!=[0,0] or state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('stack_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('IPC queue fixture cleanup failed: '+str(state))
   after=g.call('ipc_direct_status',{})
   if after.get('bound') is not False or after.get('queued')!=[0,0] or after.get('scrubbed') is not True:raise RuntimeError('second IPC channel not clean: '+str(after))
   g.call('upload_reset',{'id':queue_upload})
   # Receiver really sleeps in the kernel; the other actor continues issuing
   # VFS reads solely as a host-controlled gate, not as an IPC mailbox.
   if any(entry.get('name')=='a-wgate' for entry in files_before):raise RuntimeError('wait fixture name already exists')
   if not g.call('file_put',{'name':'a-wgate','data':'0'}).get('readback_verified'):raise RuntimeError('wait fixture setup failed')
   wait_elf=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-ipc-wait.hex').read_text().strip())
   wait_upload=stage_program(g,wait_elf)
   for stopped_waiter in (False,True):
    if stopped_waiter and not g.call('file_put',{'name':'a-wgate','data':'0'}).get('readback_verified'):raise RuntimeError('wait gate reset failed')
    sender=g.call('process_spawn_staged',{'id':wait_upload});receiver=g.call('process_spawn_staged',{'id':wait_upload})
    if sender.get('slot')!=0 or receiver.get('slot')!=1 or sender.get('boot_id')!=receiver.get('boot_id'):raise RuntimeError('wait fixture admission failed')
    g.wait_independent(lambda x:x.get('slot_states')==['ready','ready'],'wait actors ready')
    wait_args={'a_slot':0,'a_incarnation':sender['incarnation_text'],'b_slot':1,'b_incarnation':receiver['incarnation_text'],'boot_id':sender['boot_id']}
    channel_id=g.call('ipc_direct_bind',wait_args)['channel_id']
    sleeping=wait_direct(lambda x:x.get('waiting',[False,False])[1] is True,'receiver blocked')
    progress=g.call('process_status',{}).get('io_completed',[0,0])[0]
    for _ in range(30):
     activity=g.call('process_status',{})
     if activity.get('io_completed',[0,0])[0]>progress:break
     time.sleep(.2)
    else:raise RuntimeError('sender did not progress while receiver blocked: '+str(activity))
    if g.call('ipc_direct_status',{}).get('waiting',[False,False])[1] is not True:raise RuntimeError('receiver did not remain asleep')
    if not g.call('file_put',{'name':'a-wgate','data':'1'}).get('readback_verified'):raise RuntimeError('wait release failed')
    awake=wait_direct(lambda x:x.get('waiting')==[True,True] and x.get('wake_message',[0,0])[1]>sleeping.get('wake_message',[0,0])[1],'receiver woke on message and both blocked')
    if awake.get('received',[0,0])[1]!=1:raise RuntimeError('woken receiver did not receive message: '+str(awake))
    if stopped_waiter:
     stopped=g.call('process_stop',{'slot':1,'incarnation':receiver['incarnation_text'],'boot_id':receiver['boot_id']})
     if stopped.get('stopped') is not True:raise RuntimeError('could not stop sleeping receiver')
     state=g.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'waiter stopped and sleeping neighbor woke')
     if state.get('outcomes')!=[1,4] or state.get('exit_codes')!=[0,None]:raise RuntimeError('stopping waiter disturbed sender: '+str(state))
    else:
     if g.call('ipc_direct_close',{'channel_id':channel_id,'boot_id':sender['boot_id']})!={'closed':True}:raise RuntimeError('could not wake blocked tasks on close')
     state=g.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'both blocked tasks woke on close')
     if state.get('outcomes')!=[1,1] or state.get('exit_codes')!=[0,0]:raise RuntimeError('wait close wake failed: '+str(state))
    closed_wait=g.call('ipc_direct_status',{})
    if closed_wait.get('wake_close',[0,0])[0]<=awake.get('wake_close',[0,0])[0] or (not stopped_waiter and closed_wait.get('wake_close',[0,0])[1]<=awake.get('wake_close',[0,0])[1]):raise RuntimeError('missing kernel close wakes: '+str(closed_wait))
    if state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('stack_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('wait task resources not cleaned: '+str(state))
    stopped_channel=g.call('ipc_direct_status',{})
    if stopped_channel.get('bound') is not False or stopped_channel.get('waiting')!=[False,False] or stopped_channel.get('scrubbed') is not True:raise RuntimeError('wait channel cleanup invalid: '+str(stopped_channel))
   # A and B sleep on IPC while the third task is independently runnable.
   if not g.call('file_put',{'name':'a-wgate','data':'0'}).get('readback_verified') or not g.call('file_put',{'name':'c-wgate','data':'0'}).get('readback_verified'):raise RuntimeError('three-task wait gates failed')
   wa=g.call('process_spawn_staged',{'id':wait_upload});wb=g.call('process_spawn_staged',{'id':wait_upload})
   if wa.get('slot')!=0 or wb.get('slot')!=1:raise RuntimeError('wait pair admission failed')
   g.wait_independent(lambda x:x.get('slot_states')==['ready','ready'],'wait pair before third')
   wab={'a_slot':0,'a_incarnation':wa['incarnation_text'],'b_slot':1,'b_incarnation':wb['incarnation_text'],'boot_id':wa['boot_id']}
   wc_id=g.call('ipc_direct_bind',wab)['channel_id']
   before_wait=wait_direct(lambda x:x.get('waiting',[False,False])[1] is True,'receiver asleep before third')
   if not g.call('file_put',{'name':'a-wgate','data':'1'}).get('readback_verified'):raise RuntimeError('wait pair release failed')
   wait_direct(lambda x:x.get('waiting')==[True,True] and x.get('wake_message',[0,0])[1]>before_wait.get('wake_message',[0,0])[1],'both waiting before third')
   wc=g.call('process_spawn_staged',{'id':wait_upload})
   if wc.get('slot')!=2 or wc.get('boot_id')!=wa.get('boot_id'):raise RuntimeError('third wait actor admission failed')
   g.wait_independent(lambda x:x.get('all_slot_states')==['waiting','waiting','ready'] and x.get('all_io_completed',[0,0,0])[2]>0 and x.get('all_irq_counts',[0,0,0])[2]>0,'third runnable while pair sleeps')
   stopped=g.call('process_stop',{'slot':2,'incarnation':wc['incarnation_text'],'boot_id':wc['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('third stop failed')
   waiting=g.call('ipc_direct_status',{})
   if waiting.get('waiting')!=[True,True] or waiting.get('bound') is not True:raise RuntimeError('third stop disturbed sleeping pair: '+str(waiting))
   stopped=g.call('process_stop',{'slot':1,'incarnation':wb['incarnation_text'],'boot_id':wb['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('sleeping second stop failed')
   state=g.wait_independent(lambda x:x.get('all_slot_states')==['exited']*3,'sleeping neighbor resumed after third stop')
   if state.get('all_outcomes')!=[1,4,4] or state.get('all_exit_codes')!=[0,None,None] or state.get('all_data_scrubbed')!=[True]*3 or state.get('all_code_scrubbed')!=[True]*3 or state.get('all_stack_scrubbed')!=[True]*3 or state.get('frame_pool_assigned')!=0:raise RuntimeError('three-task sleep/stop cleanup invalid: '+str(state))
   if not g.call('file_delete',{'name':'c-wgate'}).get('readback_verified'):raise RuntimeError('third wait gate cleanup failed')
   g.call('upload_reset',{'id':wait_upload})
   if not g.call('file_delete',{'name':'a-wgate'}).get('readback_verified') or g.call('file_list',{})!=files_before:raise RuntimeError('wait fixture cleanup failed')
   if a.exercise_storage_fault:
    # Real CNTVCT expiration first; then freeze a test-only clock to force
    # interleavings without relying on host scheduling latency.
    for name in ('a-tgate','b-tgate','a-tphase','b-tphase'):
     if any(entry.get('name')==name for entry in files_before):raise RuntimeError('timeout fixture name already exists')
     if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('timeout fixture setup failed')
    timeout_elf=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-ipc-timeout.hex').read_text().strip())
    timeout_upload=stage_program(g,timeout_elf)
    def gate(name,value):
     if not g.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('timeout gate failed')
    def timeout_phases(a_value,b_value):
     for _ in range(60):
      av=g.call('file_get',{'name':'a-tphase'}).get('hex');bv=g.call('file_get',{'name':'b-tphase'}).get('hex')
      if av==a_value.encode().hex() and bv==b_value.encode().hex():return
      time.sleep(.2)
     raise RuntimeError('timeout phases not reached: '+str((a_value,b_value,av,bv)))
    for stop_waiter in (False,True):
     if stop_waiter:
      for name in ('a-tgate','b-tgate','a-tphase','b-tphase'):gate(name,'0')
     clock_before=g.call('ipc_direct_status',{})
     ta=g.call('process_spawn_staged',{'id':timeout_upload});tb=g.call('process_spawn_staged',{'id':timeout_upload})
     if ta.get('slot')!=0 or tb.get('slot')!=1 or ta.get('boot_id')!=tb.get('boot_id'):raise RuntimeError('timeout fixture admission failed')
     g.wait_independent(lambda x:x.get('slot_states')==['ready','ready'],'timed wait actors ready')
     targs={'a_slot':0,'a_incarnation':ta['incarnation_text'],'b_slot':1,'b_incarnation':tb['incarnation_text'],'boot_id':ta['boot_id']}
     timed_id=g.call('ipc_direct_bind',targs)['channel_id']
     timeout_phases('r','r')
     real=g.call('ipc_direct_status',{})
     if any(real.get('wake_timeout',[0,0])[i]<=clock_before.get('wake_timeout',[0,0])[i] for i in range(2)):raise RuntimeError('actual ARM counter did not expire both waits: '+str(real))
     if g.call('debug_timeout_clock_freeze',{})!={'ok':True}:raise RuntimeError('timeout clock freeze failed')
     gate('a-tgate','1');gate('b-tgate','1')
     timed=wait_direct(lambda x:x.get('waiting')==[True,True],'both timed waits asleep')
     reject(g,'debug_timeout_clock_unfreeze',{},'timed wait active')
     if stop_waiter:
      stopped=g.call('process_stop',{'slot':1,'incarnation':tb['incarnation_text'],'boot_id':tb['boot_id']})
      if stopped.get('stopped') is not True:raise RuntimeError('timed waiter stop failed')
      ended=g.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'timed stop and neighbor wake')
      if ended.get('outcomes')!=[1,4] or ended.get('exit_codes')!=[0,None]:raise RuntimeError('timed stop cleanup failed: '+str(ended))
      after=g.call('ipc_direct_status',{})
      if after.get('wake_timeout')!=timed.get('wake_timeout') or after.get('wake_close',[0,0])[0]!=timed.get('wake_close',[0,0])[0]+1:raise RuntimeError('timed stop woke wrong waiter: '+str(after))
     else:
      if g.call('debug_timeout_clock_advance',{'ns':'400000000'})!={'ok':True}:raise RuntimeError('clock advance failed')
      before_deadline=g.call('ipc_direct_status',{})
      if before_deadline.get('waiting')!=[True,True] or before_deadline.get('wake_timeout')!=timed.get('wake_timeout'):raise RuntimeError('timed waits expired early: '+str(before_deadline))
      if g.call('debug_timeout_clock_advance',{'ns':'600000000'})!={'ok':True}:raise RuntimeError('deadline advance failed')
      timeout_phases('t','t')
      expired=g.call('ipc_direct_status',{})
      if expired.get('waiting')!=[False,False] or expired.get('wake_timeout')!=[timed['wake_timeout'][i]+1 for i in range(2)]:raise RuntimeError('timed waits did not expire exactly once: '+str(expired))
      gate('b-tgate','2')
      wait_direct(lambda x:x.get('waiting',[False,False])[1] is True,'timed receiver awaiting message')
      gate('a-tgate','3')
      message=wait_direct(lambda x:x.get('queued')==[1,0] and x.get('waiting',[False,False])[1] is True and x.get('wake_message',[0,0])[1]>expired.get('wake_message',[0,0])[1],'message before deadline and queued response')
      if message.get('wake_timeout')!=expired.get('wake_timeout'):raise RuntimeError('expired wait woke twice: '+str(message))
      gate('a-tgate','4')
      both=wait_direct(lambda x:x.get('waiting')==[True,True] and x.get('received')==[1,1],'zero-duration ready and both timed waits asleep')
      if both.get('wake_timeout')!=expired.get('wake_timeout'):raise RuntimeError('unexpected timeout after message')
      if g.call('ipc_direct_close',{'channel_id':timed_id,'boot_id':ta['boot_id']})!={'closed':True}:raise RuntimeError('timed close failed')
      ended=g.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'timed close wake')
      if ended.get('outcomes')!=[1,1] or ended.get('exit_codes')!=[0,0]:raise RuntimeError('timed fixture exit failed: '+str(ended))
      after=g.call('ipc_direct_status',{})
      if after.get('wake_timeout')!=expired.get('wake_timeout') or after.get('wake_close')!=[both['wake_close'][i]+1 for i in range(2)]:raise RuntimeError('timed wait woke twice after close: '+str(after))
     if ended.get('data_scrubbed')!=[True,True] or ended.get('code_scrubbed')!=[True,True] or ended.get('stack_scrubbed')!=[True,True] or ended.get('frame_pool_assigned')!=0:raise RuntimeError('timed fixture cleanup failed: '+str(ended))
     if g.call('debug_timeout_clock_unfreeze',{})!={'ok':True}:raise RuntimeError('timeout clock restore failed')
    g.call('upload_reset',{'id':timeout_upload})
    for name in ('a-tgate','b-tgate','a-tphase','b-tphase'):
     if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('timeout fixture cleanup failed')
    if g.call('file_list',{})!=files_before:raise RuntimeError('timeout fixture left files behind')
   # Three independent loaded programs with isolated private data and stacks.
   for name in ('a-gate','b-gate','c-gate'):
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('three-task fixture name already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('three-task gate setup failed')
   three=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-three-process.hex').read_text().strip())
   three_upload=stage_program(g,three)
   actors=[g.call('process_spawn_staged',{'id':three_upload}) for _ in range(3)]
   if [a.get('slot') for a in actors]!=[0,1,2] or len({a.get('incarnation') for a in actors})!=3 or len({a.get('boot_id') for a in actors})!=1:raise RuntimeError('three independent admissions invalid: '+str(actors))
   def all_tasks(predicate,label):
    for _ in range(80):
     snapshot=g.call('process_status',{})
     if predicate(snapshot):return snapshot
     time.sleep(.2)
    raise RuntimeError('three tasks did not reach '+label+': '+str(snapshot))
   live=all_tasks(lambda x:x.get('all_slot_states')==['ready']*3 and min(x.get('all_io_completed',[0,0,0]))>=6 and min(x.get('all_irq_counts',[0,0,0]))>0,'all actors running and checked')
   if live.get('all_incarnations')!=[a['incarnation'] for a in actors] or live.get('all_mappings_verified')!=[True]*3 or live.get('frame_pool_assigned')!=9:raise RuntimeError('three-task isolation not established: '+str(live))
   reject(g,'process_spawn_staged',{'id':three_upload},'no free process slot')
   for name in ('a-gate','b-gate','c-gate'):
    if g.call('file_get',{'name':name})!={'name':name,'hex':'30'}:raise RuntimeError('cross-actor write changed gate '+name)
   previous=live['all_io_completed']
   stopped=g.call('process_stop',{'slot':0,'incarnation':actors[0]['incarnation_text'],'boot_id':actors[0]['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('three-task stop failed')
   live=all_tasks(lambda x:x.get('all_slot_states')==['exited','ready','ready'] and all(x.get('all_io_completed',[0,0,0])[i]>previous[i] for i in (1,2)),'other actors progress after stop')
   if live.get('all_outcomes',[None]*3)[0]!=4 or live.get('all_exit_codes',[0]*3)[0] is not None or live.get('all_data_scrubbed',[False]*3)[0] is not True or live.get('all_code_scrubbed',[False]*3)[0] is not True or live.get('all_stack_scrubbed',[False]*3)[0] is not True or live.get('frame_pool_assigned')!=6:raise RuntimeError('three-task stop cleanup invalid: '+str(live))
   previous=live['all_io_completed']
   replacement=g.call('process_spawn_staged',{'id':three_upload})
   if replacement.get('slot')!=0 or replacement.get('incarnation',0)<=max(a['incarnation'] for a in actors) or replacement.get('boot_id')!=actors[0]['boot_id']:raise RuntimeError('three-task readmission invalid: '+str(replacement))
   reject(g,'process_stop',{'slot':0,'incarnation':actors[0]['incarnation_text'],'boot_id':actors[0]['boot_id']},'stale process incarnation')
   live=all_tasks(lambda x:x.get('all_slot_states')==['ready']*3 and x.get('all_io_completed',[0,0,0])[0]>=6 and x.get('all_irq_counts',[0,0,0])[0]>0 and all(x.get('all_io_completed',[0,0,0])[i]>previous[i] for i in (1,2)),'replaced actor and neighbors progress')
   if live.get('all_incarnations')!=[replacement['incarnation'],actors[1]['incarnation'],actors[2]['incarnation']] or live.get('frame_pool_assigned')!=9:raise RuntimeError('three-task readmission disturbed neighbors: '+str(live))
   for name in ('a-gate','b-gate','c-gate'):
    if not g.call('file_put',{'name':name,'data':'1'}).get('readback_verified'):raise RuntimeError('three-task release failed')
   done=all_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'three-task teardown')
   if done.get('all_outcomes')!=[1]*3 or done.get('all_exit_codes')!=[0]*3 or done.get('all_data_scrubbed')!=[True]*3 or done.get('all_code_scrubbed')!=[True]*3 or done.get('all_stack_scrubbed')!=[True]*3 or done.get('frame_pool_assigned')!=0:raise RuntimeError('three-task cleanup invalid: '+str(done))
   g.call('upload_reset',{'id':three_upload})
   for name in ('a-gate','b-gate','c-gate'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('three-task gate cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('three-task fixture left files behind')
   # Three real independent ELF processes allocate the same virtual heap VA.
   # Gate names and values live in that heap and the guest checks -16 for a
   # pending heap-backed GET, as well as zeroing after free and reallocation.
   heap_gates=('a-hgate','b-hgate','c-hgate')
   for name in heap_gates:
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('heap fixture name already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('heap gate setup failed')
   image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-private-heap.hex').read_text().strip())
   heap_upload=stage_program(g,image)
   def heap_phase(name,expected):
    for _ in range(80):
     got=g.call('file_get',{'name':name}).get('hex')
     if got==expected.encode().hex():return
     time.sleep(.2)
    raise RuntimeError('heap fixture phase missing: '+str((name,expected,got)))
   heap_actors=[]
   for i,name in enumerate(heap_gates):
    task=g.call('process_spawn_staged',{'id':heap_upload});heap_actors.append(task)
    if task.get('slot')!=i:raise RuntimeError('heap actor slot mismatch: '+str(task))
    heap_phase(name,'r')
    present=all_tasks(lambda x:sum(x.get('all_heap_allocated',[False]*3))==i+1,'private heap admission')
    if present.get('all_heap_allocated')!=[j<=i for j in range(3)] or present.get('frame_pool_assigned')!=4*(i+1):raise RuntimeError('private heap admission changed neighbor: '+str(present))
   live=all_tasks(lambda x:x.get('all_heap_allocated')==[True]*3 and x.get('frame_pool_assigned')==12,'three private heaps')
   frames=live.get('all_heap_frame_index',[])
   if len(frames)!=3 or any(not isinstance(x,int) for x in frames) or len(set(frames))!=3:raise RuntimeError('heap physical frames are not distinct: '+str(live))
   previous=live['all_io_completed']
   if g.call('process_stop',{'slot':0,'incarnation':heap_actors[0]['incarnation_text'],'boot_id':heap_actors[0]['boot_id']}).get('stopped') is not True:raise RuntimeError('heap actor stop failed')
   stopped=all_tasks(lambda x:x.get('all_slot_states')==['exited','ready','ready'] and all(x.get('all_io_completed',[0]*3)[i]>previous[i] for i in (1,2)),'heap neighbors progress after stop')
   if stopped.get('all_heap_allocated')!=[False,True,True] or stopped.get('all_heap_scrubbed',[False]*3)[0] is not True or stopped.get('frame_pool_assigned')!=8:raise RuntimeError('heap stop did not clean actor resources: '+str(stopped))
   if not g.call('file_put',{'name':'a-hgate','data':'0'}).get('readback_verified'):raise RuntimeError('heap gate reset failed')
   replacement=g.call('process_spawn_staged',{'id':heap_upload})
   if replacement.get('slot')!=0 or replacement.get('incarnation',0)<=heap_actors[0]['incarnation']:raise RuntimeError('heap actor readmission failed: '+str(replacement))
   heap_phase('a-hgate','r')
   live=all_tasks(lambda x:x.get('all_heap_allocated')==[True]*3 and x.get('frame_pool_assigned')==12,'heap allocation after readmission')
   if live.get('all_heap_frame_index')!=frames:raise RuntimeError('heap frame reuse or neighbor identity changed: '+str((frames,live)))
   for name in heap_gates:
    if not g.call('file_put',{'name':name,'data':'f'}).get('readback_verified'):raise RuntimeError('heap recycle request failed')
   for name in heap_gates:heap_phase(name,'d')
   live=all_tasks(lambda x:x.get('all_heap_allocated')==[True]*3 and x.get('frame_pool_assigned')==12,'all heaps reallocated')
   for name in heap_gates:
    if not g.call('file_put',{'name':name,'data':'q'}).get('readback_verified'):raise RuntimeError('heap fixture release failed')
   done=all_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'heap fixture teardown')
   if done.get('all_outcomes')!=[1]*3 or done.get('all_exit_codes')!=[0]*3 or done.get('all_heap_allocated')!=[False]*3 or done.get('all_heap_scrubbed')!=[True]*3 or done.get('frame_pool_assigned')!=0:raise RuntimeError('heap fixture cleanup failed: '+str(done))
   g.call('upload_reset',{'id':heap_upload})
   for name in heap_gates:
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('heap gate cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('heap fixture left files behind')

   # Insert after the existing single-page heap scenario. The ELF fixture uses
   # a-pgate/b-pgate/c-pgate for process 0/1/2.
   mp_gates=('a-pgate','b-pgate','c-pgate')
   for name in mp_gates:
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('multi-page fixture name already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('multi-page gate setup failed')
   image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-multipage-heap.hex').read_text().strip())
   mp_upload=stage_program(g,image)
   def mp_phase(name,expected):
    for _ in range(80):
     got=g.call('file_get',{'name':name}).get('hex')
     if got==expected.encode().hex():return
     time.sleep(.2)
    raise RuntimeError('multi-page fixture phase missing: '+str((name,expected,got)))
   def heap_counts(status):return [sum(row) for row in status.get('all_heap_page_allocated',[])]
   mp_actors=[]
   for i,name in enumerate(mp_gates):
    task=g.call('process_spawn_staged',{'id':mp_upload});mp_actors.append(task)
    if task.get('slot')!=i:raise RuntimeError('multi-page actor slot mismatch: '+str(task))
    mp_phase(name,'r')
    present=all_tasks(lambda x:heap_counts(x)==[8 if j<=i else 0 for j in range(3)],'indexed heap admission')
    if present.get('frame_pool_assigned')!=11*(i+1):raise RuntimeError('indexed heap admission changed neighbor: '+str(present))
   live=all_tasks(lambda x:heap_counts(x)==[8]*3 and x.get('frame_pool_assigned')==33,'three eight-page heaps')
   frames=live.get('all_heap_page_frame_index',[])
   if len(frames)!=3 or any(len(row)!=8 for row in frames) or any(not isinstance(x,int) for row in frames for x in row) or len({x for row in frames for x in row})!=24:raise RuntimeError('multi-page frames not distinct: '+str(live))
   previous=live['all_io_completed']
   if g.call('process_stop',{'slot':0,'incarnation':mp_actors[0]['incarnation_text'],'boot_id':mp_actors[0]['boot_id']}).get('stopped') is not True:raise RuntimeError('multi-page actor stop failed')
   stopped=all_tasks(lambda x:x.get('all_slot_states')==['exited','ready','ready'] and all(x.get('all_io_completed',[0]*3)[i]>previous[i] for i in (1,2)),'multi-page neighbors after stop')
   if heap_counts(stopped)!=[0,8,8] or stopped.get('all_heap_page_scrubbed',[[False]*8])[0]!=[True]*8 or stopped.get('frame_pool_assigned')!=22:raise RuntimeError('multi-page stop did not clean resources: '+str(stopped))
   if not g.call('file_put',{'name':'a-pgate','data':'0'}).get('readback_verified'):raise RuntimeError('multi-page gate reset failed')
   replacement=g.call('process_spawn_staged',{'id':mp_upload})
   if replacement.get('slot')!=0 or replacement.get('incarnation',0)<=mp_actors[0]['incarnation']:raise RuntimeError('multi-page readmission failed: '+str(replacement))
   mp_phase('a-pgate','r')
   live=all_tasks(lambda x:heap_counts(x)==[8]*3 and x.get('frame_pool_assigned')==33,'multi-page after readmission')
   if live.get('all_heap_page_frame_index')!=frames:raise RuntimeError('multi-page frame reuse or neighbors changed: '+str((frames,live)))
   for name in mp_gates:
    if not g.call('file_put',{'name':name,'data':'f'}).get('readback_verified'):raise RuntimeError('multi-page recycle request failed')
   for name in mp_gates:mp_phase(name,'d')
   all_tasks(lambda x:heap_counts(x)==[8]*3 and x.get('frame_pool_assigned')==33,'all pages reallocated')
   for name in mp_gates:
    if not g.call('file_put',{'name':name,'data':'q'}).get('readback_verified'):raise RuntimeError('multi-page release failed')
   done=all_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'multi-page teardown')
   if done.get('all_outcomes')!=[1]*3 or done.get('all_exit_codes')!=[0]*3 or heap_counts(done)!=[0]*3 or done.get('all_heap_page_scrubbed')!=[[True]*8]*3 or done.get('frame_pool_assigned')!=0:raise RuntimeError('multi-page cleanup failed: '+str(done))
   g.call('upload_reset',{'id':mp_upload})
   for name in mp_gates:
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('multi-page gate cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('multi-page fixture left files behind')
   # Actual Rust Box/Vec allocations share a process-owned heap while a
   # separate HeapPage owns page 0. The gate first carries the live Vec bytes.
   alloc_gates=('a-rgate','b-rgate','c-rgate')
   for name in alloc_gates:
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('allocator fixture name already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('allocator gate setup failed')
   image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-global-allocator.hex').read_text().strip())
   alloc_upload=stage_program(g,image)
   def alloc_phase(name,expected):
    for _ in range(80):
     got=g.call('file_get',{'name':name}).get('hex')
     if got==expected.hex():return
     time.sleep(.2)
    raise RuntimeError('allocator fixture phase missing: '+str((name,expected.hex(),got)))
   def start_alloc_actor(i):
    task=g.call('process_spawn_staged',{'id':alloc_upload})
    if task.get('slot')!=i:raise RuntimeError('allocator actor slot mismatch: '+str(task))
    alloc_phase(alloc_gates[i],bytes(x^0xa5 for x in range(16)))
    if not g.call('file_put',{'name':alloc_gates[i],'data':'g'}).get('readback_verified'):raise RuntimeError('allocator payload acknowledgement failed')
    alloc_phase(alloc_gates[i],b'r')
    return task
   alloc_actors=[]
   for i in range(3):
    alloc_actors.append(start_alloc_actor(i))
    present=all_tasks(lambda x:[sum(row) for row in x.get('all_heap_page_allocated',[])]==[2 if j<=i else 0 for j in range(3)],'allocator admission')
    if present.get('frame_pool_assigned')!=5*(i+1):raise RuntimeError('allocator admission changed neighbor: '+str(present))
   live=all_tasks(lambda x:[sum(row) for row in x.get('all_heap_page_allocated',[])]==[2]*3 and x.get('frame_pool_assigned')==15,'three allocators live')
   frames=live.get('all_heap_page_frame_index',[])
   if len(frames)!=3 or any(len(row)!=8 or any(not isinstance(x,int) for x in row[:2]) or any(x is not None for x in row[2:]) for row in frames) or len({x for row in frames for x in row[:2]})!=6:raise RuntimeError('allocator pages not distinct: '+str(live))
   previous=live['all_io_completed']
   if g.call('process_stop',{'slot':0,'incarnation':alloc_actors[0]['incarnation_text'],'boot_id':alloc_actors[0]['boot_id']}).get('stopped') is not True:raise RuntimeError('allocator actor stop failed')
   stopped=all_tasks(lambda x:x.get('all_slot_states')==['exited','ready','ready'] and all(x.get('all_io_completed',[0]*3)[i]>previous[i] for i in (1,2)),'allocator neighbors after stop')
   if [sum(row) for row in stopped.get('all_heap_page_allocated',[])]!=[0,2,2] or stopped.get('all_heap_page_scrubbed',[[False]*8])[0][:2]!=[True]*2 or stopped.get('frame_pool_assigned')!=10:raise RuntimeError('allocator stop failed to reclaim pages: '+str(stopped))
   replacement=start_alloc_actor(0)
   if replacement.get('incarnation',0)<=alloc_actors[0]['incarnation']:raise RuntimeError('allocator readmission is not fresh: '+str(replacement))
   live=all_tasks(lambda x:[sum(row) for row in x.get('all_heap_page_allocated',[])]==[2]*3 and x.get('frame_pool_assigned')==15,'allocator after readmission')
   if live.get('all_heap_page_frame_index')!=frames:raise RuntimeError('allocator frame reuse or neighbors changed: '+str((frames,live)))
   for name in alloc_gates:
    if not g.call('file_put',{'name':name,'data':'q'}).get('readback_verified'):raise RuntimeError('allocator fixture release failed')
   done=all_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'allocator fixture teardown')
   if done.get('all_outcomes')!=[1]*3 or done.get('all_exit_codes')!=[0]*3 or [sum(row) for row in done.get('all_heap_page_allocated',[])]!=[0]*3 or any(row[:2]!=[True]*2 for row in done.get('all_heap_page_scrubbed',[])) or done.get('frame_pool_assigned')!=0:raise RuntimeError('allocator fixture cleanup failed: '+str(done))
   g.call('upload_reset',{'id':alloc_upload})
   for name in alloc_gates:
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('allocator gate cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('allocator fixture left files behind')

   # Requires the existing all_tasks helper and a guest with FP context support.
   fp_gates=('a-fgate','b-fgate','c-fgate')
   for name in fp_gates:
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('FP fixture name already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('FP gate setup failed')
   image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'fp-actors.hex').read_text().strip())
   fp_upload=stage_program(g,image)
   def fp_phase(name,value):
    for _ in range(80):
     got=g.call('file_get',{'name':name}).get('hex')
     if got==value.encode().hex():return
     time.sleep(.2)
    raise RuntimeError('FP phase not reached: '+str((name,value,got)))
   actors=[]
   for slot,name in enumerate(fp_gates):
    task=g.call('process_spawn_staged',{'id':fp_upload});actors.append(task)
    if task.get('slot')!=slot:raise RuntimeError('FP actor admission failed: '+str(task))
    fp_phase(name,'r')
   live=all_tasks(lambda x:x.get('all_slot_states')==['ready']*3 and min(x.get('all_irq_counts',[0]*3))>0,'three FP tasks after timer IRQ')
   baseline=live['all_irq_counts'];io_before=live['all_io_completed']
   live=all_tasks(lambda x:all(x.get('all_irq_counts',[0]*3)[i]>baseline[i] and x.get('all_io_completed',[0]*3)[i]>io_before[i] for i in range(3)),'all FP tasks survived further timer IRQ and SVC')
   previous=live['all_io_completed']
   if g.call('process_stop',{'slot':2,'incarnation':actors[2]['incarnation_text'],'boot_id':actors[2]['boot_id']}).get('stopped') is not True:raise RuntimeError('FP actor stop failed')
   all_tasks(lambda x:x.get('all_slot_states')==['ready','ready','exited'] and all(x.get('all_io_completed',[0]*3)[i]>previous[i] for i in (0,1)),'FP neighbors progress while C stopped')
   if not g.call('file_put',{'name':'c-fgate','data':'0'}).get('readback_verified'):raise RuntimeError('FP gate reset failed')
   replacement=g.call('process_spawn_staged',{'id':fp_upload})
   if replacement.get('slot')!=2 or replacement.get('incarnation',0)<=actors[2]['incarnation']:raise RuntimeError('FP readmission failed: '+str(replacement))
   fp_phase('c-fgate','r') # New EL0 instance verified all FP state starts at zero.
   args={'a_slot':0,'a_incarnation':actors[0]['incarnation_text'],'b_slot':1,'b_incarnation':actors[1]['incarnation_text'],'boot_id':actors[0]['boot_id']}
   bound=g.call('ipc_direct_bind',args);channel=bound['channel_id']
   if not g.call('file_put',{'name':'b-fgate','data':'w'}).get('readback_verified'):raise RuntimeError('FP wait request failed')
   for _ in range(80):
    status=g.call('ipc_direct_status',{})
    if status.get('all_waiting',[False]*3)[1] is True:break
    time.sleep(.2)
   else:raise RuntimeError('FP actor did not sleep: '+str(status))
   if not g.call('file_put',{'name':'a-fgate','data':'s'}).get('readback_verified'):raise RuntimeError('FP send request failed')
   fp_phase('a-fgate','d');fp_phase('b-fgate','d')
   status=g.call('ipc_direct_status',{})
   if status.get('all_waiting',[True]*3)[1] or status.get('all_wake_message',[0]*3)[1]<1:raise RuntimeError('FP waiter did not wake: '+str(status))
   for name in fp_gates:
    if not g.call('file_put',{'name':name,'data':'q'}).get('readback_verified'):raise RuntimeError('FP release failed')
   done=all_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'FP task teardown')
   if done.get('all_outcomes')!=[1]*3 or done.get('all_exit_codes')!=[0]*3 or done.get('frame_pool_assigned')!=0:raise RuntimeError('FP task failed or leaked: '+str(done))
   g.call('upload_reset',{'id':fp_upload})
   for name in fp_gates:
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('FP gate cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('FP fixture left files behind')
   if a.exercise_storage_fault:
    # A single direct channel can choose any two of three live actors.
    for name in ('a-igate','b-igate','c-igate'):
     if any(entry.get('name')==name for entry in files_before):raise RuntimeError('IPC-three gate already exists')
     if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('IPC-three gate setup failed')
    ipc_three=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-ipc-three.hex').read_text().strip())
    ipc_upload=stage_program(g,ipc_three)
    ia=g.call('process_spawn_staged',{'id':ipc_upload});ib=g.call('process_spawn_staged',{'id':ipc_upload});ic=g.call('process_spawn_staged',{'id':ipc_upload})
    if [ia.get('slot'),ib.get('slot'),ic.get('slot')]!=[0,1,2] or len({ia.get('boot_id'),ib.get('boot_id'),ic.get('boot_id')})!=1:raise RuntimeError('IPC-three admission failed')
    all_tasks(lambda x:x.get('all_slot_states')==['ready']*3 and min(x.get('all_io_completed',[0]*3))>0,'IPC-three actors active')
    if g.call('debug_timeout_clock_freeze',{})!={'ok':True}:raise RuntimeError('IPC-three clock freeze failed')
    bind12={'a_slot':2,'a_incarnation':ic['incarnation_text'],'b_slot':1,'b_incarnation':ib['incarnation_text'],'boot_id':ia['boot_id']}
    reject(g,'ipc_direct_bind',dict(bind12,a_slot=3),'invalid channel slots')
    old_id=g.call('ipc_direct_bind',bind12)['channel_id']
    if g.call('ipc_direct_status',{}).get('slots')!=[1,2]:raise RuntimeError('IPC endpoints not sorted')
    def ipcg(name,value):
     if not g.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('IPC-three gate failed')
    def ipcphase(name,value):
     for _ in range(80):
      got=g.call('file_get',{'name':name}).get('hex')
      if got==value.encode().hex():return
      time.sleep(.2)
     raise RuntimeError('IPC-three phase not reached: '+str((name,value,got)))
    for name in ('a-igate','b-igate','c-igate'):ipcg(name,'1')
    ipcphase('a-igate','v')  # Nonmember tested peer/send/recv/wait/timed.
    wait_direct(lambda x:x.get('all_waiting',[False]*3)[2] is True,'slot 2 timed waiter')
    ipcg('b-igate','2')
    full=wait_direct(lambda x:x.get('slots')==[1,2] and x.get('queued')==[0,2] and x.get('full_rejections',[0,0])[0]>0,'1-to-2 full FIFO')
    ipcg('c-igate','3')
    consumed=wait_direct(lambda x:x.get('received')==[0,2] and x.get('all_waiting',[False]*3)[2] is True,'slot 2 consumed FIFO and reblocked')
    baseline_timeout=consumed['all_wake_timeout']
    if g.call('debug_timeout_clock_advance',{'ns':'10000000000'})!={'ok':True}:raise RuntimeError('IPC-three timeout advance failed')
    ipcphase('c-igate','t')
    timed=wait_direct(lambda x:x.get('all_waiting',[False]*3)[2] is True and x.get('all_wake_timeout',[0]*3)[2]==baseline_timeout[2]+1,'slot 2 timeout and reblock')
    stopped=g.call('process_stop',{'slot':2,'incarnation':ic['incarnation_text'],'boot_id':ic['boot_id']})
    if stopped.get('stopped') is not True:raise RuntimeError('IPC member stop failed')
    wait_direct(lambda x:x.get('bound') is False and x.get('all_waiting',[False]*3)[2] is False,'member stop revoked channel')
    ipcphase('b-igate','d')
    ipcg('c-igate','0')
    ic2=g.call('process_spawn_staged',{'id':ipc_upload})
    if ic2.get('slot')!=2 or ic2.get('incarnation',0)<=ic['incarnation']:raise RuntimeError('IPC member readmission failed')
    all_tasks(lambda x:x.get('all_slot_states')==['ready']*3,'IPC member readmitted')
    stale={'a_slot':2,'a_incarnation':ic['incarnation_text'],'b_slot':0,'b_incarnation':ia['incarnation_text'],'boot_id':ia['boot_id']}
    reject(g,'ipc_direct_bind',stale,'stale process incarnation')
    bind02=dict(stale,a_incarnation=ic2['incarnation_text'])
    new_id=g.call('ipc_direct_bind',bind02)['channel_id']
    if int(new_id)<=int(old_id) or g.call('ipc_direct_status',{}).get('slots')!=[0,2]:raise RuntimeError('0-to-2 binding invalid')
    reject(g,'ipc_direct_close',{'channel_id':old_id,'boot_id':ia['boot_id']},'stale direct channel')
    ipcg('b-igate','7');ipcphase('b-igate','v')  # Slot 1 is now the outsider.
    ipcg('a-igate','4');ipcg('c-igate','4')
    wait_direct(lambda x:x.get('all_waiting',[False]*3)[2] is True,'readmitted slot 2 waiting')
    ipcg('a-igate','5');ipcphase('c-igate','n')
    wait_direct(lambda x:x.get('slots')==[0,2] and x.get('all_waiting')==[True,False,True] and x.get('sent')==[1,0] and x.get('received')==[0,1],'0-to-2 delivery and both blocked')
    if g.call('ipc_direct_close',{'channel_id':new_id,'boot_id':ia['boot_id']})!={'closed':True}:raise RuntimeError('IPC 0-to-2 close failed')
    all_tasks(lambda x:x.get('all_slot_states')==['exited','ready','exited'],'0-to-2 members exited after close')
    ipcg('b-igate','9')
    finished=all_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'IPC-three cleanup')
    if finished.get('all_outcomes')!=[1]*3 or finished.get('all_exit_codes')!=[0]*3 or finished.get('all_data_scrubbed')!=[True]*3 or finished.get('all_code_scrubbed')!=[True]*3 or finished.get('all_stack_scrubbed')!=[True]*3 or finished.get('frame_pool_assigned')!=0:raise RuntimeError('IPC-three task cleanup invalid: '+str(finished))
    final_channel=g.call('ipc_direct_status',{})
    if final_channel.get('bound') is not False or final_channel.get('queued')!=[0,0] or final_channel.get('scrubbed') is not True:raise RuntimeError('IPC-three channel cleanup invalid: '+str(final_channel))
    if g.call('debug_timeout_clock_unfreeze',{})!={'ok':True}:raise RuntimeError('IPC-three clock restore failed')
    g.call('upload_reset',{'id':ipc_upload})
    for name in ('a-igate','b-igate','c-igate'):
     if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('IPC-three gate cleanup failed')
    if g.call('file_list',{})!=files_before:raise RuntimeError('IPC-three fixture left files behind')
   # Two distinct channels may now remain live at the same time.
   for name in ('a-mgate','b-mgate','c-mgate'):
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('multi-channel gate already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('multi-channel gate setup failed')
   multi=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-ipc-multi.hex').read_text().strip())
   multi_upload=stage_program(g,multi)
   def multi_tasks(predicate,label):
    for _ in range(80):
     snapshot=g.call('process_status',{})
     if predicate(snapshot):return snapshot
     time.sleep(.2)
    raise RuntimeError('multi-channel tasks did not reach '+label+': '+str(snapshot))
   def multi_channel(snapshot,id):
    return next((channel for channel in snapshot.get('channels',[]) if channel.get('channel_id')==int(id)),None)
   def multi_status(predicate,label):
    for _ in range(80):
     snapshot=g.call('ipc_direct_status',{})
     if predicate(snapshot):return snapshot
     time.sleep(.2)
    raise RuntimeError('multi-channel status not reached '+label+': '+str(snapshot))
   def put_multi(name,ab,ac,stage):
    data=(int(ab).to_bytes(8,'little')+int(ac).to_bytes(8,'little')+bytes([stage])).hex()
    if not g.call('file_put_hex',{'name':name,'hex':data}).get('readback_verified'):raise RuntimeError('multi-channel gate write failed')
   def spawn_multi():
    tasks=[g.call('process_spawn_staged',{'id':multi_upload}) for _ in range(3)]
    if [t.get('slot') for t in tasks]!=[0,1,2] or len({t.get('boot_id') for t in tasks})!=1:raise RuntimeError('multi-channel admission failed')
    multi_tasks(lambda x:x.get('all_slot_states')==['ready']*3 and min(x.get('all_io_completed',[0]*3))>0,'all actors ready')
    return tasks
   def bind_multi(tasks,a_slot,b_slot):
    return g.call('ipc_direct_bind',{'a_slot':a_slot,'a_incarnation':tasks[a_slot]['incarnation_text'],'b_slot':b_slot,'b_incarnation':tasks[b_slot]['incarnation_text'],'boot_id':tasks[0]['boot_id']})['channel_id']
   def close_multi(tasks,id):
    if g.call('ipc_direct_close',{'channel_id':id,'boot_id':tasks[0]['boot_id']})!={'closed':True}:raise RuntimeError('multi-channel close failed')
   def reset_multi():
    for name in ('a-mgate','b-mgate','c-mgate'):
     if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('multi-channel gate reset failed')
   def finished_multi(expected_outcomes,expected_exits,label):
    state=multi_tasks(lambda x:x.get('all_slot_states')==['exited']*3,label)
    if state.get('all_outcomes')!=expected_outcomes or state.get('all_exit_codes')!=expected_exits or state.get('all_data_scrubbed')!=[True]*3 or state.get('all_code_scrubbed')!=[True]*3 or state.get('all_stack_scrubbed')!=[True]*3 or state.get('frame_pool_assigned')!=0:raise RuntimeError('multi-channel cleanup invalid: '+str(state))
    status=g.call('ipc_direct_status',{})
    if status.get('channels')!=[] or status.get('scrubbed') is not True:raise RuntimeError('multi-channel queues remain: '+str(status))
   tasks=spawn_multi();ab=bind_multi(tasks,0,1);ac=bind_multi(tasks,0,2)
   both=g.call('ipc_direct_status',{})
   if len(both.get('channels',[]))!=2 or both.get('channel_id') is not None or both.get('slots') is not None:raise RuntimeError('ambiguous channel view invalid: '+str(both))
   reject(g,'ipc_direct_bind',{'a_slot':0,'a_incarnation':tasks[0]['incarnation_text'],'b_slot':1,'b_incarnation':tasks[1]['incarnation_text'],'boot_id':tasks[0]['boot_id']},'direct channel already bound')
   reject(g,'ipc_direct_bind',{'a_slot':1,'a_incarnation':tasks[1]['incarnation_text'],'b_slot':2,'b_incarnation':tasks[2]['incarnation_text'],'boot_id':tasks[0]['boot_id']},'direct channel capacity reached')
   for name in ('a-mgate','b-mgate','c-mgate'):put_multi(name,ab,ac,1)
   multi_status(lambda x:all((c:=multi_channel(x,id)) is not None and c.get('sent')==[1,1] and c.get('received')==[1,1] for id in (ab,ac)),'both channels exchanged distinct messages')
   close_multi(tasks,ab)
   survivor=multi_status(lambda x:(c:=multi_channel(x,ac)) is not None and c.get('sent')==[2,2] and c.get('received')==[2,2] and multi_channel(x,ab) is None,'surviving channel progresses after other close')
   stopped=g.call('process_stop',{'slot':1,'incarnation':tasks[1]['incarnation_text'],'boot_id':tasks[1]['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('multi-channel member stop failed')
   if not g.call('file_put',{'name':'b-mgate','data':'0'}).get('readback_verified'):raise RuntimeError('multi-channel B reset failed')
   old_b=tasks[1];replacement=g.call('process_spawn_staged',{'id':multi_upload})
   if replacement.get('slot')!=1 or replacement.get('incarnation',0)<=old_b['incarnation']:raise RuntimeError('multi-channel B readmission failed')
   reject(g,'ipc_direct_bind',{'a_slot':0,'a_incarnation':tasks[0]['incarnation_text'],'b_slot':1,'b_incarnation':old_b['incarnation_text'],'boot_id':tasks[0]['boot_id']},'stale process incarnation')
   tasks[1]=replacement;ab2=bind_multi(tasks,0,1)
   reject(g,'ipc_direct_close',{'channel_id':ab,'boot_id':tasks[0]['boot_id']},'stale direct channel')
   put_multi('a-mgate',ab2,ac,2);put_multi('b-mgate',ab2,ac,2)
   multi_status(lambda x:(c:=multi_channel(x,ac)) is not None and c.get('sent')==[3,3] and c.get('received')==[3,3] and (b:=multi_channel(x,ab2)) is not None and b.get('sent')==[1,1] and b.get('received')==[1,1],'readmitted channel and survivor progress')
   close_multi(tasks,ab2);close_multi(tasks,ac);finished_multi([1,1,1],[0,0,0],'multi-channel re-admission teardown')
   # Stopping the common member must revoke both channels.
   reset_multi();tasks=spawn_multi();ab=bind_multi(tasks,0,1);ac=bind_multi(tasks,0,2)
   for name in ('a-mgate','b-mgate','c-mgate'):put_multi(name,ab,ac,3)
   multi_status(lambda x:all((c:=multi_channel(x,id)) is not None and c.get('sent')==[1,1] and c.get('received')==[1,1] for id in (ab,ac)) and x.get('all_waiting',[False]*3)[0] is True,'shared member waiting on first channel')
   if g.call('process_stop',{'slot':0,'incarnation':tasks[0]['incarnation_text'],'boot_id':tasks[0]['boot_id']}).get('stopped') is not True:raise RuntimeError('shared member stop failed')
   finished_multi([4,1,1],[None,0,0],'shared member stop cleanup')
   # A waits on AB. A message and closure on AC must not wake that wait.
   reset_multi();tasks=spawn_multi();ab=bind_multi(tasks,0,1);ac=bind_multi(tasks,0,2)
   for name in ('a-mgate','b-mgate','c-mgate'):put_multi(name,ab,ac,4)
   waiting=multi_status(lambda x:x.get('all_waiting',[False]*3)[0] is True,'A sleeping on AB')
   before_message=waiting['all_wake_message'][0];before_close=waiting['all_wake_close'][0]
   # Both senders acknowledge that they read stage 4 before either gate can
   # be overwritten with stage 5, independent of host or IRQ timing.
   for _ in range(80):
    b_ready=g.call('file_get',{'name':'b-mgate'}).get('hex');c_ready=g.call('file_get',{'name':'c-mgate'}).get('hex')
    if b_ready=='72' and c_ready=='72':break
    time.sleep(.2)
   else:raise RuntimeError('multi-channel stage 4 readiness missing: '+str((b_ready,c_ready)))
   put_multi('c-mgate',ab,ac,5)
   unrelated=multi_status(lambda x:(c:=multi_channel(x,ac)) is not None and c.get('queued')==[1,0],'unrelated channel queued')
   if unrelated.get('all_waiting',[False]*3)[0] is not True or unrelated.get('all_wake_message',[0]*3)[0]!=before_message:raise RuntimeError('unrelated message woke A: '+str(unrelated))
   close_multi(tasks,ac)
   closed=g.call('ipc_direct_status',{})
   if closed.get('all_waiting',[False]*3)[0] is not True or closed.get('all_wake_close',[0]*3)[0]!=before_close:raise RuntimeError('unrelated close woke A: '+str(closed))
   put_multi('b-mgate',ab,ac,5)
   multi_status(lambda x:(c:=multi_channel(x,ab)) is not None and c.get('received')==[1,0] and x.get('all_wake_message',[0]*3)[0]==before_message+1,'matching channel woke A')
   close_multi(tasks,ab);finished_multi([1,1,1],[0,0,0],'unrelated-channel wake isolation')
   g.call('upload_reset',{'id':multi_upload})
   for name in ('a-mgate','b-mgate','c-mgate'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('multi-channel gate cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('multi-channel fixture left files behind')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Draft: the actual MCP schemas and errors must be reconciled with the guest.
  # QEMU-only disposable clones; no physical block device or host directory.
  tree_disk_path=Path(tmp)/'tree-volume.raw';create(tree_disk_path,disk.read_bytes())
  def protected_program_sectors(path):
   data=path.read_bytes();return (data[20*512:40*512],data[64*512:130*512])
  def reserved_tree_sectors(path):return path.read_bytes()[256*512:768*512]
  def non_tree_sectors(path):
   data=path.read_bytes();return (data[:256*512],data[768*512:])
  def chunked_tree_read(vm,path,size,generation):
   result=bytearray()
   for offset in range(0,size,512):
    reply=vm.call('tree_read',{'path':path,'offset':offset,'capacity':min(512,size-offset),'generation':generation})
    if reply.get('path')!=path or reply.get('offset')!=offset or reply.get('size')!=size or reply.get('generation')!=generation:raise RuntimeError('wrong tree read metadata: '+str(reply))
    data=bytes.fromhex(reply['hex'])
    if len(data)!=min(512,size-offset):raise RuntimeError('wrong tree read length')
    result.extend(data)
   eof=vm.call('tree_read',{'path':path,'offset':size,'capacity':1,'generation':generation})
   if eof.get('hex')!='':raise RuntimeError('tree EOF returned data')
   return bytes(result)
  payload=bytes(((i*37+11)^(i>>8)^(i>>13))&255 for i in range(16*1024))
  t=Guest(a.qemu,guest,tree_disk_path,tmp,58,out,a.transport)
  try:
   t.progress('before provisioned tree volume',timeout_image=True,expected_exit_code=0)
   old_programs=protected_program_sectors(tree_disk_path);old_non_tree=non_tree_sectors(tree_disk_path)
   old_document=t.call('document_snapshot',{});old_files=t.call('file_list',{})
   status=t.call('tree_status',{})
   if status.get('slots')!=['blank','blank']:raise RuntimeError('tree region was not blank: '+str(status))
   provision=t.call('tree_provision',{'dedicated':True})
   if provision.get('readback_verified') is not True or provision.get('generation')!='1':raise RuntimeError('tree provision failed: '+str(provision))
   generation=provision['generation']
   reject(t,'tree_mkdir',{'path':'/missing/child','generation':generation},'tree entry not found')
   for path in ('/human','/human/projects','/agent'):
    update=t.call('tree_mkdir',{'path':path,'generation':generation})
    if update.get('readback_verified') is not True:raise RuntimeError('tree mkdir not verified: '+str(update))
    generation=update['generation']
   # A sealed upload may not overwrite a newer generation.
   stale=t.call('tree_upload_begin',{'path':'/human/projects/stale.bin','size':4,'checksum':fnv64(b'test'),'generation':generation})['id']
   if t.call('tree_upload_append',{'id':stale,'offset':0,'hex':b'test'.hex()}).get('received')!=4:raise RuntimeError('stale upload staging failed')
   if t.call('tree_upload_seal',{'id':stale})!={'sealed':True}:raise RuntimeError('stale upload sealing failed')
   update=t.call('tree_mkdir',{'path':'/other','generation':generation})
   if update.get('readback_verified') is not True:raise RuntimeError('tree revision update failed')
   old_generation=generation;generation=update['generation']
   reject(t,'tree_upload_commit',{'id':stale},'stale tree generation')
   if t.call('tree_upload_reset',{'id':stale})!={'reset':True}:raise RuntimeError('stale upload reset failed')
   reject(t,'tree_list',{'generation':old_generation},'stale tree generation')
   bad=t.call('tree_upload_begin',{'path':'/human/projects/bad.bin','size':4,'checksum':fnv64(b'other'),'generation':generation})['id']
   if t.call('tree_upload_append',{'id':bad,'offset':0,'hex':b'test'.hex()}).get('received')!=4:raise RuntimeError('checksum test staging failed')
   reject(t,'tree_upload_seal',{'id':bad},'tree upload checksum mismatch')
   reject(t,'tree_upload_commit',{'id':bad},'tree upload not sealed')
   if t.call('tree_upload_reset',{'id':bad})!={'reset':True}:raise RuntimeError('bad upload reset failed')
   begin=t.call('tree_upload_begin',{'path':'/human/projects/data.bin','size':len(payload),'checksum':fnv64(payload),'generation':generation})
   token=begin['id']
   reject(t,'tree_upload_append',{'id':token,'offset':1,'hex':'00'},'tree upload offset mismatch')
   for offset in range(0,len(payload),512):
    reply=t.call('tree_upload_append',{'id':token,'offset':offset,'hex':payload[offset:offset+512].hex()})
    if reply.get('received')!=min(offset+512,len(payload)):raise RuntimeError('wrong tree upload progress: '+str(reply))
   if t.call('tree_upload_seal',{'id':token})!={'sealed':True}:raise RuntimeError('tree upload failed sealing')
   update=t.call('tree_upload_commit',{'id':token})
   if update.get('readback_verified') is not True:raise RuntimeError('tree commit not verified: '+str(update))
   generation=update['generation']
   listed=t.call('tree_list',{'generation':generation})
   expected_paths={'/','/human','/human/projects','/agent','/other','/human/projects/data.bin'}
   if {e['path'] for e in listed.get('entries',[])}!=expected_paths:raise RuntimeError('tree hierarchy differs: '+str(listed))
   if next((e for e in listed['entries'] if e['path']=='/human/projects/data.bin'),None)!={'path':'/human/projects/data.bin','kind':'file','size':len(payload)}:raise RuntimeError('tree file metadata differs')
   if chunked_tree_read(t,'/human/projects/data.bin',len(payload),generation)!=payload:raise RuntimeError('tree bytes differ')
   if t.call('document_snapshot',{})!=old_document or t.call('file_list',{})!=old_files:raise RuntimeError('separate tree operation modified legacy workspace')
   if protected_program_sectors(tree_disk_path)!=old_programs:raise RuntimeError('tree commit modified program sectors')
   if non_tree_sectors(tree_disk_path)!=old_non_tree:raise RuntimeError('tree commit modified sectors outside reserved tree area')
   traces.append(t.trace);states.append(t.call('tree_status',{}))
  finally:t.close()
  t=Guest(a.qemu,guest,tree_disk_path,tmp,59,out,a.transport)
  try:
   t.progress('after tree volume restart',timeout_image=True,expected_exit_code=0)
   after_startup_non_tree=non_tree_sectors(tree_disk_path)
   opened=t.call('tree_open',{'dedicated':True})
   if opened.get('generation')!=generation:raise RuntimeError('tree generation not restored: '+str(opened))
   if chunked_tree_read(t,'/human/projects/data.bin',len(payload),generation)!=payload:raise RuntimeError('tree bytes lost on reboot')
   if t.call('document_snapshot',{})!=old_document or t.call('file_list',{})!=old_files:raise RuntimeError('legacy workspace changed after tree reboot')
   if protected_program_sectors(tree_disk_path)!=old_programs:raise RuntimeError('tree restart modified program sectors')
   if non_tree_sectors(tree_disk_path)!=after_startup_non_tree:raise RuntimeError('tree open/read modified sectors outside reserved tree area')
   traces.append(t.trace);states.append(t.call('tree_status',{}))
  finally:t.close()

  # A valid latest copy remains readable beside an unknown or invalid older copy,
  # but opening must never repair the companion or authorize writes.
  tree_image=tree_disk_path.read_bytes();slot_starts=(256*512,512*512)
  generations=[int.from_bytes(tree_image[offset+8:offset+16],'little') for offset in slot_starts]
  if any(tree_image[offset:offset+8]!=b'TREE0001' for offset in slot_starts) or max(generations)!=int(generation) or generations[0]==generations[1]:raise RuntimeError('tree recovery fixture lacks two distinct valid generations')
  older=generations.index(min(generations))
  for index,(kind,error) in enumerate((('unknown','unknown tree region'),('invalid','tree region needs repair'))):
   damaged=bytearray(tree_image);offset=slot_starts[older]
   if kind=='unknown':damaged[offset:offset+8]=b'FOREIGN!'
   else:damaged[offset+28]=0 # recognized record with missing commit marker
   damaged_path=Path(tmp)/('tree-degraded-'+kind+'.raw');create(damaged_path,damaged)
   t=Guest(a.qemu,guest,damaged_path,tmp,61+index,out,a.transport)
   try:
    t.progress('before degraded tree open',timeout_image=True,expected_exit_code=0)
    before_tree=reserved_tree_sectors(damaged_path);before_programs=protected_program_sectors(damaged_path);before_non_tree=non_tree_sectors(damaged_path)
    before_open=t.call('tree_status',{})
    if before_open.get('state')!='degraded' or before_open.get('opened') is not False or before_open.get('writable') is not False:raise RuntimeError('unopened degraded status mismatch: '+str(before_open))
    opened=t.call('tree_open',{'dedicated':True})
    if opened.get('generation')!=generation or opened.get('degraded') is not True or opened.get('writable') is not False:raise RuntimeError('degraded tree chose wrong generation: '+str(opened))
    status=t.call('tree_status',{})
    if status.get('state')!='degraded' or status.get('opened') is not True or status.get('writable') is not False or status.get('slots',[None,None])[older]!=kind:raise RuntimeError('degraded tree status mismatch: '+str(status))
    if chunked_tree_read(t,'/human/projects/data.bin',len(payload),generation)!=payload:raise RuntimeError('degraded tree read lost bytes')
    if '/human/projects/data.bin' not in {e['path'] for e in t.call('tree_list',{'generation':generation}).get('entries',[])}:raise RuntimeError('degraded tree list lost file')
    reject(t,'tree_mkdir',{'path':'/forbidden','generation':generation},error)
    reject(t,'tree_upload_begin',{'path':'/human/projects/forbidden.bin','size':4,'checksum':fnv64(b'test'),'generation':generation},error)
    if reserved_tree_sectors(damaged_path)!=before_tree or protected_program_sectors(damaged_path)!=before_programs:raise RuntimeError('degraded tree read/refusal modified protected sectors')
    if non_tree_sectors(damaged_path)!=before_non_tree:raise RuntimeError('degraded tree read/refusal modified outside sectors')
    traces.append(t.trace);states.append(t.call('tree_status',{}))
   finally:t.close()

  # Neither duplicate generations nor two invalid records may be selected.
  for index,kind in enumerate(('both-invalid','duplicate-generation')):
   damaged=bytearray(tree_image)
   if kind=='both-invalid':
    for offset in slot_starts:damaged[offset+28]=0
   else:
    offset=slot_starts[older]
    damaged[offset+8:offset+16]=int(generation).to_bytes(8,'little')
    n=int.from_bytes(damaged[offset+16:offset+20],'little')
    digest=int(fnv64(damaged[offset+8:offset+20]+damaged[offset+512:offset+512+n]),16)
    damaged[offset+20:offset+28]=digest.to_bytes(8,'little')
   damaged_path=Path(tmp)/('tree-selection-'+kind+'.raw');create(damaged_path,damaged)
   t=Guest(a.qemu,guest,damaged_path,tmp,63+index,out,a.transport)
   try:
    t.progress('before invalid tree selection',timeout_image=True,expected_exit_code=0)
    before=damaged_path.read_bytes();status=t.call('tree_status',{})
    expected_slots=['invalid','invalid'] if kind=='both-invalid' else ['valid','valid']
    if status.get('state')!='invalid' or status.get('slots')!=expected_slots or status.get('generation') is not None or status.get('opened') is not False:raise RuntimeError('invalid tree selection was not refused: '+str(status))
    reject(t,'tree_open',{'dedicated':True},'tree region needs repair')
    reject(t,'tree_provision',{'dedicated':True},'tree region needs repair')
    if damaged_path.read_bytes()!=before:raise RuntimeError('invalid tree selection changed virtual disk')
    traces.append(t.trace);states.append(t.call('tree_status',{}))
   finally:t.close()

  # Refusal before mutating a foreign signature in the reserved region.
  foreign=bytearray(disk.read_bytes());foreign[256*512:256*512+8]=b'FOREIGN!'
  foreign_path=Path(tmp)/'tree-foreign.raw';create(foreign_path,foreign)
  t=Guest(a.qemu,guest,foreign_path,tmp,60,out,a.transport)
  try:
   t.progress('before unknown tree region',timeout_image=True,expected_exit_code=0)
   before_tree=reserved_tree_sectors(foreign_path);before_programs=protected_program_sectors(foreign_path)
   status=t.call('tree_status',{})
   if status.get('slots',[None,None])[0]!='unknown':raise RuntimeError('unknown tree region not detected: '+str(status))
   reject(t,'tree_provision',{'dedicated':True},'unknown tree region')
   if reserved_tree_sectors(foreign_path)!=before_tree or protected_program_sectors(foreign_path)!=before_programs:raise RuntimeError('refused unknown tree altered protected sectors')
   traces.append(t.trace);states.append(t.call('tree_status',{}))
  finally:t.close()
  # Clone the separately provisioned synthetic tree; prepare per-slot files,
  # reboot without opening it, then run actual independent EL0 readers.
  tree_el0_path=Path(tmp)/'tree-el0.raw';create(tree_el0_path,tree_disk_path.read_bytes())
  tree_el0_programs=protected_program_sectors(tree_el0_path)
  tree_el0_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-tree-read.hex').read_text().strip())
  data_a=bytes(((i*31+3)^(i>>8))&255 for i in range(513))
  data_b=bytes(((i*19+79)^(i>>8))&255 for i in range(513))
  def put_tree(vm,path,data,generation):
   token=vm.call('tree_upload_begin',{'path':path,'size':len(data),'checksum':fnv64(data),'generation':generation})['id']
   for offset in range(0,len(data),512):
    reply=vm.call('tree_upload_append',{'id':token,'offset':offset,'hex':data[offset:offset+512].hex()})
    if reply.get('received')!=min(offset+512,len(data)):raise RuntimeError('tree EL0 upload progress invalid')
   if vm.call('tree_upload_seal',{'id':token})!={'sealed':True}:raise RuntimeError('tree EL0 upload seal failed')
   commit=vm.call('tree_upload_commit',{'id':token})
   if commit.get('readback_verified') is not True:raise RuntimeError('tree EL0 upload did not commit: '+str(commit))
   return commit['generation']
  e=Guest(a.qemu,guest,tree_el0_path,tmp,66,out,a.transport)
  try:
   e.progress('before preparing scoped tree files',timeout_image=True,expected_exit_code=0)
   opened=e.call('tree_open',{'dedicated':True});tree_generation=opened['generation']
   if opened.get('writable') is not True:raise RuntimeError('tree EL0 setup unexpectedly read-only')
   for path in ('/a','/a/dir','/b','/b/dir'):
    result=e.call('tree_mkdir',{'path':path,'generation':tree_generation})
    if result.get('readback_verified') is not True:raise RuntimeError('tree EL0 mkdir not committed')
    tree_generation=result['generation']
   tree_generation=put_tree(e,'/a/data',data_a,tree_generation)
   tree_generation=put_tree(e,'/b/data',data_b,tree_generation)
   if chunked_tree_read(e,'/a/data',len(data_a),tree_generation)!=data_a or chunked_tree_read(e,'/b/data',len(data_b),tree_generation)!=data_b:raise RuntimeError('tree EL0 setup bytes mismatch')
   if protected_program_sectors(tree_el0_path)!=tree_el0_programs:raise RuntimeError('tree EL0 setup changed program regions')
   traces.append(e.trace);states.append(e.call('tree_status',{}))
  finally:e.close()
  e=Guest(a.qemu,guest,tree_el0_path,tmp,67,out,a.transport)
  try:
   e.progress('before scoped EL0 tree readers',timeout_image=True,expected_exit_code=0);e.wait_boot_identity()
   status=e.call('tree_status',{})
   if status.get('state')!='unopened' or status.get('generation')!=tree_generation:raise RuntimeError('tree EL0 volume unexpectedly opened after restart: '+str(status))
   gates=('a-tgate','b-tgate');phases=('a-tphase','b-tphase')
   for name in gates+phases:
    if any(item.get('name')==name for item in e.call('file_list',{})):raise RuntimeError('tree EL0 gate name already exists')
   def gate(name,flag):
    if not e.call('file_put',{'name':name,'data':flag}).get('readback_verified'):raise RuntimeError('tree EL0 gate failed')
   def phase(name,value):
    for _ in range(100):
     result=e.call('file_get',{'name':name}).get('hex')
     if result==value.encode().hex():return
     time.sleep(.2)
    raise RuntimeError('tree EL0 phase missing: '+str((name,value,result)))
   gate(gates[0],'c')
   if not e.call('file_put',{'name':phases[0],'data':'0'}).get('readback_verified'):raise RuntimeError('tree EL0 closed phase setup failed')
   program=stage_program(e,tree_el0_image)
   closed=e.call('process_spawn_staged',{'id':program})
   phase(phases[0],'c')
   state=e.wait_independent(lambda x:x.get('all_slot_states',[None]*3)[0]=='exited','closed tree reader exit')
   if state.get('all_outcomes',[None]*3)[0]!=1 or state.get('all_exit_codes',[None]*3)[0]!=0:raise RuntimeError('closed tree reader failed: '+str(state))
   opened=e.call('tree_open',{'dedicated':True})
   if opened.get('generation')!=tree_generation or opened.get('writable') is not True:raise RuntimeError('tree EL0 open failed: '+str(opened))
   for name in gates:gate(name,'0')
   for name in phases:
    if not e.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('tree EL0 phase reset failed')
   actors=[e.call('process_spawn_staged',{'id':program}) for _ in range(2)]
   if [actor.get('slot') for actor in actors]!=[0,1] or actors[0]['incarnation']<=closed['incarnation']:raise RuntimeError('tree EL0 reader admissions failed: '+str(actors))
   for name in phases:phase(name,'r')
   ready=e.wait_independent(lambda x:x.get('slot_states')==['ready','ready'] and min(x.get('irq_counts',[0,0]))>0,'both scoped tree readers active')
   if ready.get('frame_pool_assigned')!=6:raise RuntimeError('tree EL0 reader mapping count invalid: '+str(ready))
   for name in gates:gate(name,'1')
   for name in phases:phase(name,'d')
   done=e.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'scoped tree readers exit')
   if done.get('outcomes')!=[1,1] or done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('scoped tree readers failed or leaked: '+str(done))
   if chunked_tree_read(e,'/a/data',len(data_a),tree_generation)!=data_a or chunked_tree_read(e,'/b/data',len(data_b),tree_generation)!=data_b:raise RuntimeError('tree EL0 readers changed stored bytes')
   if protected_program_sectors(tree_el0_path)!=tree_el0_programs:raise RuntimeError('tree EL0 reads changed program regions')
   e.call('upload_reset',{'id':program});traces.append(e.trace);states.append(e.call('tree_status',{}))
  finally:e.close()
  # Two real EL0 writers, same starting tree generation, per-incarnation stages.
  # Separate disposable copies test 512-byte then full 16-KiB staged writes.
  write_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-tree-write.hex').read_text().strip())
  def run_tree_writers(big,boot_index):
   write_disk=Path(tmp)/('tree-el0-write-large.raw' if big else 'tree-el0-write-small.raw')
   create(write_disk,tree_disk_path.read_bytes());program_sectors=protected_program_sectors(write_disk)
   e=Guest(a.qemu,guest,write_disk,tmp,boot_index,out,a.transport)
   try:
    e.progress('before tree writers',timeout_image=True,expected_exit_code=0);e.wait_boot_identity()
    opened=e.call('tree_open',{'dedicated':True});write_generation=opened['generation']
    if opened.get('writable') is not True:raise RuntimeError('tree write volume not writable')
    for path in ('/a','/b'):
     update=e.call('tree_mkdir',{'path':path,'generation':write_generation})
     if update.get('readback_verified') is not True:raise RuntimeError('tree writer mkdir failed')
     write_generation=update['generation']
    gates=('a-ugate','b-ugate');phases=('a-uphase','b-uphase')
    names=[entry['name'] for entry in e.call('file_list',{})]
    if any(name in names for name in gates+phases):raise RuntimeError('tree writer gate already exists')
    def gate(name,value):
     if not e.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('tree writer gate failed')
    def phase(name,value):
     for _ in range(900):
      result=e.call('file_get',{'name':name}).get('hex')
      if result==value.encode().hex():return
      slot=0 if name[0]=='a' else 1;state=e.call('process_status',{})
      if state.get('all_slot_states',[None]*3)[slot]=='exited':raise RuntimeError('tree writer exited before phase: '+str((name,value,result,state)))
      time.sleep(.2)
     raise RuntimeError('tree writer phase timeout: '+str((name,value,result)))
    for name in gates:gate(name,'L' if big else '0')
    for name in phases:gate(name,'0')
    program=stage_program(e,write_image)
    actors=[e.call('process_spawn_staged',{'id':program}) for _ in range(2)]
    if [x.get('slot') for x in actors]!=[0,1]:raise RuntimeError('tree writer admissions failed: '+str(actors))
    for name in phases:phase(name,'r')
    ready=e.wait_independent(lambda x:x.get('slot_states')==['ready','ready'] and min(x.get('irq_counts',[0,0]))>0,'both tree writers staged')
    if ready.get('frame_pool_assigned')!=6:raise RuntimeError('tree writer frame accounting invalid: '+str(ready))
    gate(gates[0],'a');phase(phases[0],'d')
    after_a=e.call('tree_status',{})
    if int(after_a['generation'])!=int(write_generation)+1:raise RuntimeError('first tree writer did not advance generation: '+str(after_a))
    gate(gates[1],'b');phase(phases[1],'e')
    after_b=e.call('tree_status',{})
    final_generation=after_b['generation']
    if int(final_generation)!=int(write_generation)+2:raise RuntimeError('second tree writer did not retry at new generation: '+str(after_b))
    gate(gates[0],'x');phase(phases[0],'s')
    if e.call('process_stop',{'slot':0,'incarnation':actors[0]['incarnation_text'],'boot_id':actors[0]['boot_id']}).get('stopped') is not True:raise RuntimeError('tree writer stop failed')
    before=e.call('process_status',{})['all_io_completed'][1]
    e.wait_independent(lambda x:x.get('all_slot_states',[None]*3)[:2]==['exited','ready'] and x.get('all_io_completed',[0]*3)[1]>before,'neighbor after staged writer stop')
    gate(gates[0],'p');gate(phases[0],'0')
    replacement=e.call('process_spawn_staged',{'id':program})
    if replacement.get('slot')!=0 or replacement.get('incarnation',0)<=actors[0]['incarnation']:raise RuntimeError('tree writer readmission failed: '+str(replacement))
    phase(phases[0],'p');gate(gates[1],'q')
    done=e.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'tree writers done')
    if done.get('outcomes')!=[1,1] or done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('tree writer exit failure: '+str(done))
    size_a=16384 if big else 512;size_b=513
    data_a=bytes(((i*37+5)^(i>>8))&255 for i in range(size_a));data_b=bytes(((i*23+77)^(i>>8))&255 for i in range(size_b))
    if chunked_tree_read(e,'/a/write',size_a,final_generation)!=data_a or chunked_tree_read(e,'/b/write',size_b,final_generation)!=data_b:raise RuntimeError('tree writer content differs')
    entries={entry['path'] for entry in e.call('tree_list',{'generation':final_generation}).get('entries',[])}
    if any(path in entries for path in ('/a/abort','/b/abort','/a/scrat')):raise RuntimeError('uncommitted stage was published: '+str(entries))
    if protected_program_sectors(write_disk)!=program_sectors:raise RuntimeError('tree writers changed program regions')
    e.call('upload_reset',{'id':program});traces.append(e.trace);states.append(e.call('tree_status',{}))
   finally:e.close()
   e=Guest(a.qemu,guest,write_disk,tmp,boot_index+1,out,a.transport)
   try:
    e.progress('after tree writer reboot',timeout_image=True,expected_exit_code=0)
    opened=e.call('tree_open',{'dedicated':True})
    if opened.get('generation')!=final_generation:raise RuntimeError('tree writer generation lost on reboot')
    if chunked_tree_read(e,'/a/write',size_a,final_generation)!=data_a or chunked_tree_read(e,'/b/write',size_b,final_generation)!=data_b:raise RuntimeError('tree writer content lost on reboot')
    if protected_program_sectors(write_disk)!=program_sectors:raise RuntimeError('tree writer reboot changed program regions')
    traces.append(e.trace);states.append(e.call('tree_status',{}))
   finally:e.close()
  run_tree_writers(False,68)
  run_tree_writers(True,70)
  # Two EL0 actors mutate only their own directory scopes and discover versions.
  dir_disk=Path(tmp)/'tree-el0-dir.raw';create(dir_disk,tree_disk_path.read_bytes())
  dir_program_sectors=protected_program_sectors(dir_disk)
  dir_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-tree-dir.hex').read_text().strip())
  e=Guest(a.qemu,guest,dir_disk,tmp,72,out,a.transport)
  try:
   e.progress('before EL0 directory operations',timeout_image=True,expected_exit_code=0);e.wait_boot_identity()
   revision=e.call('tree_open',{'dedicated':True})['generation']
   for path in ('/a','/b'):
    result=e.call('tree_mkdir',{'path':path,'generation':revision})
    if result.get('readback_verified') is not True:raise RuntimeError('directory actor root creation failed')
    revision=result['generation']
   revision=put_tree(e,'/a/erase',b'a',revision);revision=put_tree(e,'/b/erase',b'b',revision)
   gates=('a-mgate','b-mgate');phases=('a-mphase','b-mphase')
   names=[entry['name'] for entry in e.call('file_list',{})]
   if any(name in names for name in gates+phases):raise RuntimeError('directory test gate already exists')
   def directory_gate(name,value):
    if not e.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('directory gate write failed')
   def directory_phase(name,value):
    for _ in range(160):
     result=e.call('file_get',{'name':name}).get('hex')
     if result==value.encode().hex():return
     slot=0 if name[0]=='a' else 1;state=e.call('process_status',{})
     if state.get('all_slot_states',[None]*3)[slot]=='exited':raise RuntimeError('directory actor exited before phase: '+str((name,value,result,state)))
     time.sleep(.2)
    raise RuntimeError('directory phase timed out: '+str((name,value,result)))
   for name in gates+phases:directory_gate(name,'0')
   program=stage_program(e,dir_image)
   actors=[e.call('process_spawn_staged',{'id':program}) for _ in range(2)]
   if [x.get('slot') for x in actors]!=[0,1]:raise RuntimeError('directory actor admissions failed: '+str(actors))
   for name in phases:directory_phase(name,'r')
   ready=e.wait_independent(lambda x:x.get('slot_states')==['ready','ready'] and min(x.get('irq_counts',[0,0]))>0,'both directory actors ready')
   if ready.get('frame_pool_assigned')!=6:raise RuntimeError('directory actor frame count invalid: '+str(ready))
   directory_gate(gates[0],'a');directory_phase(phases[0],'d')
   after_a=e.call('tree_status',{})
   if int(after_a['generation'])!=int(revision)+6:raise RuntimeError('directory actor A generation count differs: '+str(after_a))
   directory_gate(gates[1],'b');directory_phase(phases[1],'d')
   after_b=e.call('tree_status',{});final_generation=after_b['generation']
   if int(final_generation)!=int(revision)+12:raise RuntimeError('directory actor B generation count differs: '+str(after_b))
   listing=e.call('tree_list',{'generation':final_generation})['entries'];paths={x['path'] for x in listing}
   if not {'/a/keep','/b/keep'}.issubset(paths) or paths.intersection({'/a/test','/a/test/sub','/b/test','/b/test/sub','/a/erase','/b/erase'}):raise RuntimeError('EL0 directory changes incorrect: '+str(listing))
   for name in gates:directory_gate(name,'q')
   done=e.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'directory actors exited')
   if done.get('outcomes')!=[1,1] or done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('directory actor cleanup failed: '+str(done))
   if protected_program_sectors(dir_disk)!=dir_program_sectors:raise RuntimeError('directory operations modified program regions')
   e.call('upload_reset',{'id':program});traces.append(e.trace);states.append(e.call('tree_status',{}))
  finally:e.close()
  e=Guest(a.qemu,guest,dir_disk,tmp,73,out,a.transport)
  try:
   e.progress('after EL0 directory reboot',timeout_image=True,expected_exit_code=0)
   if e.call('tree_open',{'dedicated':True}).get('generation')!=final_generation:raise RuntimeError('directory generation did not survive reboot')
   listing=e.call('tree_list',{'generation':final_generation})['entries'];paths={x['path'] for x in listing}
   if not {'/a/keep','/b/keep'}.issubset(paths) or paths.intersection({'/a/test','/a/test/sub','/b/test','/b/test/sub','/a/erase','/b/erase'}):raise RuntimeError('EL0 directory changes lost after reboot: '+str(listing))
   if protected_program_sectors(dir_disk)!=dir_program_sectors:raise RuntimeError('directory reboot modified program regions')
   traces.append(e.trace);states.append(e.call('tree_status',{}))
  finally:e.close()
  # Two processes enumerate only immediate children in their own tree scopes.
  list_disk=Path(tmp)/'tree-el0-list.raw';create(list_disk,tree_disk_path.read_bytes())
  list_program_sectors=protected_program_sectors(list_disk)
  list_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-tree-list.hex').read_text().strip())
  listing_generation=None
  for boot in (0,1):
   e=Guest(a.qemu,guest,list_disk,tmp,74+boot,out,a.transport)
   try:
    e.progress('EL0 tree enumeration',timeout_image=True,expected_exit_code=0);e.wait_boot_identity()
    opened=e.call('tree_open',{'dedicated':True});revision=opened['generation']
    if boot==0:
     for path in ('/a','/a/d','/a/d/branch','/b','/b/d','/b/d/branch'):
      result=e.call('tree_mkdir',{'path':path,'generation':revision})
      if result.get('readback_verified') is not True:raise RuntimeError('list test mkdir failed')
      revision=result['generation']
     for first in ('a','b'):
      for suffix,data in (('aa',b'a'),('branch/hidden',b'h'),('zz',b'z'),('z'*32,b'l')):
       revision=put_tree(e,'/'+first+'/d/'+suffix,data,revision)
     listing_generation=revision
    elif revision!=listing_generation:raise RuntimeError('enumeration tree revision did not survive reboot')
    gates=('a-lgate','b-lgate');phases=('a-lphase','b-lphase')
    names=[entry['name'] for entry in e.call('file_list',{})]
    if any(name in names for name in gates+phases):raise RuntimeError('enumeration gate already exists')
    def list_gate(name,value):
     if not e.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('enumeration gate write failed')
    def list_phase(name,value):
     for _ in range(160):
      result=e.call('file_get',{'name':name}).get('hex')
      if result==value.encode().hex():return
      slot=0 if name[0]=='a' else 1;state=e.call('process_status',{})
      if state.get('all_slot_states',[None]*3)[slot]=='exited':raise RuntimeError('enumerator exited before phase: '+str((name,value,result,state)))
      time.sleep(.2)
     raise RuntimeError('enumeration phase timed out: '+str((name,value,result)))
    for name in gates+phases:list_gate(name,'0')
    program=stage_program(e,list_image)
    actors=[e.call('process_spawn_staged',{'id':program}) for _ in range(2)]
    if [x.get('slot') for x in actors]!=[0,1]:raise RuntimeError('enumeration admissions failed: '+str(actors))
    for name in phases:list_phase(name,'r')
    ready=e.wait_independent(lambda x:x.get('slot_states')==['ready','ready'] and min(x.get('irq_counts',[0,0]))>0,'both enumerators ready')
    if ready.get('frame_pool_assigned')!=6:raise RuntimeError('enumerator frame count invalid: '+str(ready))
    for name in gates:list_gate(name,'1')
    for name in phases:list_phase(name,'d')
    done=e.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'enumerators exited')
    if done.get('outcomes')!=[1,1] or done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('enumerator cleanup failed: '+str(done))
    if e.call('tree_status',{})['generation']!=listing_generation:raise RuntimeError('enumeration mutated tree')
    if protected_program_sectors(list_disk)!=list_program_sectors:raise RuntimeError('enumeration changed program regions')
    e.call('upload_reset',{'id':program})
    for name in gates+phases:
     if not e.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('enumeration gate cleanup failed')
    traces.append(e.trace);states.append(e.call('tree_status',{}))
   finally:e.close()
  # Two scoped EL0 actors exercise 255-byte tree paths and unchanged 16-byte ABIs.
  path_disk=Path(tmp)/'tree-el0-long-path.raw';create(path_disk,tree_disk_path.read_bytes())
  path_program_sectors=protected_program_sectors(path_disk)
  path_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-tree-paths.hex').read_text().strip())
  e=Guest(a.qemu,guest,path_disk,tmp,76,out,a.transport)
  try:
   e.progress('before long tree paths',timeout_image=True,expected_exit_code=0);e.wait_boot_identity()
   revision=e.call('tree_open',{'dedicated':True})['generation'];document_before=e.call('document_snapshot',{})
   for path in ('/a','/b'):
    result=e.call('tree_mkdir',{'path':path,'generation':revision})
    if result.get('readback_verified') is not True:raise RuntimeError('long-path actor root creation failed')
    revision=result['generation']
   gates=('a-pgate','b-pgate');phases=('a-pphase','b-pphase')
   names=[entry['name'] for entry in e.call('file_list',{})]
   if any(name in names for name in gates+phases):raise RuntimeError('long-path test gate already exists')
   def path_gate(name,binding,flag):
    raw=int(binding).to_bytes(8,'little')+flag.encode()
    if not e.call('file_put_hex',{'name':name,'hex':raw.hex()}).get('readback_verified'):raise RuntimeError('long-path gate write failed')
   def path_phase(name,value):
    for _ in range(160):
     result=e.call('file_get',{'name':name}).get('hex')
     if result==value.encode().hex():return
     slot=0 if name[0]=='a' else 1;state=e.call('process_status',{})
     if state.get('all_slot_states',[None]*3)[slot]=='exited':raise RuntimeError('long-path actor exited before phase: '+str((name,value,result,state)))
     time.sleep(.2)
    raise RuntimeError('long-path phase timed out: '+str((name,value,result)))
   for name in gates:path_gate(name,0,'0')
   for name in phases:
    if not e.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('long-path phase setup failed')
   program=stage_program(e,path_image);actors=[e.call('process_spawn_staged',{'id':program}) for _ in range(2)]
   if [actor.get('slot') for actor in actors]!=[0,1]:raise RuntimeError('long-path actor admissions failed: '+str(actors))
   for name in phases:path_phase(name,'r')
   bindings=[]
   for slot,role in enumerate(('human','assistant')):
    bindings.append(e.call('document_bind',{'slot':slot,'incarnation':actors[slot]['incarnation_text'],'boot_id':actors[slot]['boot_id'],'role':role})['binding_id'])
   path_gate(gates[0],bindings[0],'1');path_phase(phases[0],'d')
   after_a=e.call('tree_status',{})
   if int(after_a['generation'])!=int(revision)+5:raise RuntimeError('long-path A generation invalid: '+str(after_a))
   path_gate(gates[1],bindings[1],'1');path_phase(phases[1],'d')
   after_b=e.call('tree_status',{});final_generation=after_b['generation']
   if int(final_generation)!=int(revision)+10:raise RuntimeError('long-path B generation invalid: '+str(after_b))
   expected_paths=['/'+first+'/'+'p'*30 for first in ('a','b')]
   for first,path in zip(('a','b'),expected_paths):
    expected=bytes(((i*19)^ord(first))&255 for i in range(65))
    if chunked_tree_read(e,path,65,final_generation)!=expected:raise RuntimeError('long-path kept file differs')
   listing=e.call('tree_list',{'generation':final_generation})['entries'];paths={entry['path'] for entry in listing}
   if not set(expected_paths).issubset(paths) or any('/'+first+'/'+'k'*252 in paths or '/'+first+'/'+'f'*30 in paths for first in ('a','b')):raise RuntimeError('long-path create/delete result differs: '+str(listing))
   for name,binding in zip(gates,bindings):path_gate(name,binding,'q')
   done=e.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'long-path actors exited')
   if done.get('outcomes')!=[1,1] or done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('long-path actor cleanup failed: '+str(done))
   if e.call('document_snapshot',{})!=document_before or protected_program_sectors(path_disk)!=path_program_sectors:raise RuntimeError('long-path operations modified legacy workspace/programs')
   e.call('upload_reset',{'id':program});traces.append(e.trace);states.append(e.call('tree_status',{}))
  finally:e.close()
  e=Guest(a.qemu,guest,path_disk,tmp,77,out,a.transport)
  try:
   e.progress('after long-path reboot',timeout_image=True,expected_exit_code=0)
   if e.call('tree_open',{'dedicated':True}).get('generation')!=final_generation:raise RuntimeError('long-path tree revision not restored')
   for first,path in zip(('a','b'),expected_paths):
    expected=bytes(((i*19)^ord(first))&255 for i in range(65))
    if chunked_tree_read(e,path,65,final_generation)!=expected:raise RuntimeError('long-path file lost on reboot')
   listing=e.call('tree_list',{'generation':final_generation})['entries'];paths={entry['path'] for entry in listing}
   if not set(expected_paths).issubset(paths) or any('/'+first+'/'+'k'*252 in paths or '/'+first+'/'+'f'*30 in paths for first in ('a','b')):raise RuntimeError('long-path deletion not retained after reboot')
   if e.call('document_snapshot',{})!=document_before or protected_program_sectors(path_disk)!=path_program_sectors:raise RuntimeError('long-path reboot changed legacy workspace/programs')
   traces.append(e.trace);states.append(e.call('tree_status',{}))
  finally:e.close()
  # Isolated process surfaces use a fresh disposable virtual disk.
  from surface_scenario import run_surface_scenario
  surface_disk=Path(tmp)/'surface-process.raw';create(surface_disk,bytes(SIZE))
  surface_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-surface.hex').read_text().strip())
  surface_guest=Guest(a.qemu,guest,surface_disk,tmp,78,out,a.transport)
  try:
   run_surface_scenario(surface_guest,out,surface_image,stage_program)
   traces.append(surface_guest.trace);states.append(surface_guest.call('process_status',{}))
   (out/'surface-qmp-evidence.json').write_text(json.dumps(surface_guest.qmp_trace,indent=2)+'\n')
  finally:surface_guest.close()
  # Two disposable VMs exercise keyboard input and absence of the device.
  from surface_key_scenario import run_keyboard_scenario
  key_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-surface-key.hex').read_text().strip())
  for keyboard in (True,False):
   key_disk=Path(tmp)/('surface-key-'+str(keyboard)+'.raw');create(key_disk,bytes(SIZE))
   key_guest=Guest(a.qemu,guest,key_disk,tmp,79 if keyboard else 80,out,a.transport,keyboard=keyboard)
   try:
    run_keyboard_scenario(key_guest,key_image,stage_program,keyboard)
    traces.append(key_guest.trace);states.append(key_guest.call('process_status',{}))
    (out/('surface-key-qmp-'+str(keyboard)+'.json')).write_text(json.dumps(key_guest.qmp_trace,indent=2)+'\n')
   finally:key_guest.close()
  # Two VM boots, one disposable clone; no host file or raw device is attached.
  chunk_disk=Path(tmp)/'chunk-offset.raw';create(chunk_disk,disk.read_bytes())
  chunk_image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-chunk-io.hex').read_text().strip())
  expected_chunk=bytearray((i&255)^0x5a for i in range(512));expected_chunk[30:40]=b'CHANGED123'
  def chunk_phase(guest,value):
   for _ in range(100):
    got=guest.call('file_get',{'name':'a-ogate'}).get('hex')
    if got==value.encode().hex():return
    time.sleep(.2)
   raise RuntimeError('chunk phase missing: '+str((value,got)))
  for boot in (0,1):
   chunk=Guest(a.qemu,guest,chunk_disk,tmp,56+boot,out,a.transport)
   try:
    chunk.progress('before offset task',timeout_image=True,expected_exit_code=0);chunk.wait_boot_identity()
    if boot==0:
     for name in ('a-chunk','a-ogate','a-empty'):
      if any(entry.get('name')==name for entry in chunk.call('file_list',{})):raise RuntimeError('offset fixture name already exists')
     if not chunk.call('file_put',{'name':'a-ogate','data':'0'}).get('readback_verified'):raise RuntimeError('offset gate setup failed')
    else:
     if chunk.call('file_get',{'name':'a-chunk'})!={'name':'a-chunk','hex':expected_chunk.hex()}:raise RuntimeError('offset write not restored on reboot')
     if chunk.call('file_get',{'name':'a-empty'})!={'name':'a-empty','hex':''}:raise RuntimeError('empty file not restored on reboot')
     if not chunk.call('file_put',{'name':'a-ogate','data':'r'}).get('readback_verified'):raise RuntimeError('offset restore gate failed')
    chunk_upload=stage_program(chunk,chunk_image)
    task=chunk.call('process_spawn_staged',{'id':chunk_upload})
    if task.get('slot')!=0:raise RuntimeError('offset process admission failed: '+str(task))
    if boot==0:
     chunk_phase(chunk,'w')
     if chunk.call('file_get',{'name':'a-chunk'})!={'name':'a-chunk','hex':expected_chunk.hex()}:raise RuntimeError('offset writes or suffix mismatch')
     if chunk.call('process_stop',{'slot':0,'incarnation':task['incarnation_text'],'boot_id':task['boot_id']}).get('stopped') is not True:raise RuntimeError('offset process stop failed')
     state=chunk.wait_independent(lambda x:x.get('all_slot_states',[None]*3)[0]=='exited','offset writer stopped')
     if state.get('all_outcomes',[None]*3)[0]!=4 or state.get('frame_pool_assigned')!=0:raise RuntimeError('offset writer cleanup failed: '+str(state))
    else:
     chunk_phase(chunk,'v')
     state=chunk.wait_independent(lambda x:x.get('all_slot_states',[None]*3)[0]=='exited','offset reader exited')
     if state.get('all_outcomes',[None]*3)[0]!=1 or state.get('all_exit_codes',[None]*3)[0]!=0 or state.get('frame_pool_assigned')!=0:raise RuntimeError('offset reader failed: '+str(state))
     if chunk.call('file_get',{'name':'a-chunk'})!={'name':'a-chunk','hex':expected_chunk.hex()}:raise RuntimeError('offset read unexpectedly modified file')
    chunk.call('upload_reset',{'id':chunk_upload});traces.append(chunk.trace);states.append(chunk.call('storage_status',{}))
   finally:chunk.close()
  # Separate disposable image: document PUT persists before process ACK.
  doc_disk=Path(tmp)/'document-process.raw';create(doc_disk,disk.read_bytes())
  d=Guest(a.qemu,guest,doc_disk,tmp,54,out,a.transport)
  try:
   d.progress('before document-bound tasks',timeout_image=True,expected_exit_code=0);d.wait_boot_identity()
   before_doc=d.call('document_snapshot',{})
   if any(key in before_doc[copy] for copy in ('base','human','assistant') for key in ('hproof','aproof')):raise RuntimeError('document test keys already exist')
   if any(before_doc[copy].get('title')!='base' for copy in ('human','assistant')):raise RuntimeError('unexpected initial document title')
   for name in ('a-dgate','b-dgate'):
    if not d.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('document gate setup failed')
   image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-document-process.hex').read_text().strip())
   upload=stage_program(d,image)
   human=d.call('process_spawn_staged',{'id':upload});assistant=d.call('process_spawn_staged',{'id':upload})
   if [human.get('slot'),assistant.get('slot')]!=[0,1] or human.get('boot_id')!=assistant.get('boot_id'):raise RuntimeError('document task admission failed')
   def doc_phase(name,expected):
    for _ in range(80):
     got=d.call('file_get',{'name':name}).get('hex')
     if got==expected.encode().hex():return
     time.sleep(.2)
    raise RuntimeError('document fixture phase not reached: '+str((name,expected,got)))
   def doc_gate(name,id,stage):
    raw=int(id).to_bytes(8,'little')+bytes([stage])
    if not d.call('file_put_hex',{'name':name,'hex':raw.hex()}).get('readback_verified'):raise RuntimeError('document gate write failed')
   doc_phase('a-dgate','d');doc_phase('b-dgate','d') # Each observed -13 before binding.
   ah={'slot':0,'incarnation':human['incarnation_text'],'boot_id':human['boot_id'],'role':'human'}
   ba={'slot':1,'incarnation':assistant['incarnation_text'],'boot_id':assistant['boot_id'],'role':'assistant'}
   ahid=d.call('document_bind',ah)['binding_id']
   reject(d,'document_bind',dict(ba,role='human'),'document role already bound')
   bid=d.call('document_bind',ba)['binding_id']
   if int(ahid)==int(bid):raise RuntimeError('document binding IDs not distinct')
   reject(d,'document_bind',ah,'process already bound')
   bindings=d.call('document_bindings_status',{})
   expected_bindings=[{'slot':0,'incarnation':human['incarnation_text'],'binding_id':ahid,'role':'human'}, {'slot':1,'incarnation':assistant['incarnation_text'],'binding_id':bid,'role':'assistant'}]
   if set(bindings)!={'bindings','submitted','completed'} or bindings.get('bindings')!=expected_bindings:raise RuntimeError('document bindings invalid: '+str(bindings))
   doc_gate('a-dgate',ahid,1);doc_gate('b-dgate',bid,1)
   for _ in range(80):
    snapshot=d.call('document_snapshot',{})
    if snapshot['human'].get('hproof')=='H' and snapshot['assistant'].get('aproof')=='A':break
    time.sleep(.2)
   else:raise RuntimeError('two document writes not applied: '+str(snapshot))
   if snapshot['assistant'].get('hproof') is not None or snapshot['human'].get('aproof') is not None:raise RuntimeError('edits crossed copies')
   state=d.call('process_status',{});b_io=state['all_io_completed'][1]
   stopped=d.call('process_stop',{'slot':0,'incarnation':human['incarnation_text'],'boot_id':human['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('document human stop failed')
   for _ in range(80):
    state=d.call('process_status',{})
    if state.get('all_slot_states',[None]*3)[:2]==['exited','ready'] and state.get('all_io_completed',[0]*3)[1]>b_io:break
    time.sleep(.2)
   else:raise RuntimeError('assistant did not progress after human stop: '+str(state))
   bindings=d.call('document_bindings_status',{})
   if bindings.get('bindings')!=expected_bindings[1:]:raise RuntimeError('stopped actor binding not revoked: '+str(bindings))
   if not d.call('file_put',{'name':'a-dgate','data':'0'}).get('readback_verified'):raise RuntimeError('document gate reset failed')
   replacement=d.call('process_spawn_staged',{'id':upload})
   if replacement.get('slot')!=0 or replacement.get('incarnation',0)<=human['incarnation']:raise RuntimeError('document human readmission failed')
   doc_phase('a-dgate','d');doc_gate('a-dgate',ahid,2);doc_phase('a-dgate','s') # Re-admitted ELF observed -116.
   reject(d,'document_unbind',{'binding_id':ahid,'boot_id':human['boot_id']},'stale document binding')
   fresh=d.call('document_bind',dict(ah,incarnation=replacement['incarnation_text']))['binding_id']
   if int(fresh)<=int(ahid):raise RuntimeError('document binding ID did not advance')
   bindings=d.call('document_bindings_status',{})
   expected_fresh=[{'slot':0,'incarnation':replacement['incarnation_text'],'binding_id':fresh,'role':'human'},expected_bindings[1]]
   if bindings.get('bindings')!=expected_fresh:raise RuntimeError('document re-binding invalid: '+str(bindings))
   # Capture the neighbor immediately before the final human write; earlier
   # progress while re-admitting A does not prove progress after this point.
   b_io=d.call('process_status',{})['all_io_completed'][1]
   doc_gate('a-dgate',fresh,3)
   state=d.wait_independent(lambda x:x.get('slot_states',[None,None])[0]=='exited' and x.get('io_completed',[0,0])[1]>b_io,'rebound document writer exited while assistant progresses')
   if state.get('outcomes',[None,None])[0]!=1 or state.get('exit_codes',[None,None])[0]!=0:raise RuntimeError('rebound document writer failed: '+str(state))
   snapshot=d.call('document_snapshot',{})
   expected=json.loads(json.dumps(before_doc));expected['human']['hproof']='H2';expected['assistant']['aproof']='A'
   if snapshot!=expected:raise RuntimeError('document copies not as expected: '+str(snapshot))
   merged=d.call('document_merge',{});fields={field['key']:field for field in merged['fields']}
   if fields.get('hproof',{}).get('value')!='H2' or fields['hproof'].get('origin')!='human' or fields.get('aproof',{}).get('value')!='A' or fields['aproof'].get('origin')!='assistant':raise RuntimeError('document provenance invalid: '+str(merged))
   stopped=d.call('process_stop',{'slot':1,'incarnation':assistant['incarnation_text'],'boot_id':assistant['boot_id']})
   if stopped.get('stopped') is not True:raise RuntimeError('assistant stop failed')
   d.wait_independent(lambda x:x.get('slot_states')==['exited','exited'],'document tasks stopped')
   if d.call('document_bindings_status',{}).get('bindings')!=[]:raise RuntimeError('document bindings survived task exit')
   d.call('upload_reset',{'id':upload})
   traces.append(d.trace);states.append(d.call('storage_status',{}))
  finally:d.close()
  # No file changes after last document PUT: same disk must restore the result.
  d=Guest(a.qemu,guest,doc_disk,tmp,55,out,a.transport)
  try:
   d.progress('after document process restart',timeout_image=True,expected_exit_code=0)
   if d.call('document_snapshot',{})!=expected:raise RuntimeError('document edits did not restore')
   merged=d.call('document_merge',{});fields={field['key']:field for field in merged['fields']}
   if fields.get('hproof',{}).get('origin')!='human' or fields.get('aproof',{}).get('origin')!='assistant':raise RuntimeError('document origins not restored')
   traces.append(d.trace);states.append(d.call('storage_status',{}))
  finally:d.close()
  # No NIC: the same Rust program must see -19 without requiring a peer.
  network_elf=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-net-roundtrip.hex').read_text().strip())
  g=Guest(a.qemu,guest,disk,tmp,52,out,a.transport)
  try:
   g.progress('before missing network',timeout_image=True,expected_exit_code=0);g.wait_boot_identity()
   network_status=g.call('network_status',{})
   if network_status.get('available') is not False or network_status.get('ports')!=[False,False]:raise RuntimeError('network unexpectedly available: '+str(network_status))
   upload_id=stage_program(g,network_elf)
   task=g.call('process_spawn_staged',{'id':upload_id})
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','missing network refusal')
   if task.get('slot')!=0 or state.get('outcomes',[None,None])[0]!=1 or state.get('exit_codes',[None,None])[0]!=0 or state.get('frame_pool_assigned')!=0:raise RuntimeError('missing network was not refused: '+str(state))
   g.call('upload_reset',{'id':upload_id})
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  # Both virtual NICs use only a QEMU in-process hub, with no host backend.
  g=Guest(a.qemu,guest,disk,tmp,53,out,a.transport,network=True)
  try:
   g.progress('before VirtIO network tasks',timeout_image=True,expected_exit_code=0);g.wait_boot_identity()
   network_status=g.call('network_status',{})
   if network_status.get('available') is not True or network_status.get('ports')!=[True,True]:raise RuntimeError('two VirtIO network ports unavailable: '+str(network_status))
   files_before=g.call('file_list',{})
   for name in ('a-nphase','b-nphase','a-ntrig'):
    if any(entry.get('name')==name for entry in files_before):raise RuntimeError('network fixture name already exists')
    if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('network fixture setup failed')
   upload_id=stage_program(g,network_elf)
   task_a=g.call('process_spawn_staged',{'id':upload_id})
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='ready','network sender waits for peer')
   if task_a.get('slot')!=0 or state.get('frame_pool_assigned')!=3 or g.call('network_status',{}).get('tx_submitted')!=[0,0]:raise RuntimeError('network sender ran without peer')
   task_b=g.call('process_spawn_staged',{'id':upload_id})
   if task_b.get('slot')!=1 or task_a.get('incarnation')==task_b.get('incarnation') or task_a.get('boot_id')!=task_b.get('boot_id'):raise RuntimeError('network process admission invalid')
   def wait_phases(a_value,b_value):
    for _ in range(60):
     one=g.call('file_get',{'name':'a-nphase'}).get('hex');two=g.call('file_get',{'name':'b-nphase'}).get('hex')
     if one==a_value.encode().hex() and two==b_value.encode().hex():return
     time.sleep(.2)
    raise RuntimeError('network tasks did not reach phases '+repr((a_value,b_value,one,two)))
   def wait_network(predicate,label):
    for _ in range(60):
     snapshot=g.call('network_status',{})
     if predicate(snapshot):return snapshot
     time.sleep(.2)
    raise RuntimeError('network did not reach '+label+': '+str(snapshot))
   wait_phases('p','p')  # Both guests observed -13 before a grant.
   args={'a_slot':0,'a_incarnation':task_a['incarnation_text'],'b_slot':1,'b_incarnation':task_b['incarnation_text'],'boot_id':task_a['boot_id']}
   reject(g,'network_grant_open',dict(args,b_slot=0),'invalid network slots')
   reject(g,'network_grant_open',dict(args,a_incarnation=str(task_a['incarnation']+1)),'stale process incarnation')
   wrong_boot=('0' if args['boot_id'][0]!='0' else '1')+args['boot_id'][1:]
   reject(g,'network_grant_open',dict(args,boot_id=wrong_boot),'stale boot identity')
   generation=g.call('network_grant_open',args)['generation']
   wait_phases('1','p')
   wait_network(lambda x:x.get('rx_delivered')==[1,1],'first exchange')
   if a.exercise_storage_fault:
    if g.call('debug_network_hold_rx',{'slot':1,'hold':True})!={'held':True}:raise RuntimeError('failed to hold RX for stale-frame proof')
   if not g.call('file_put',{'name':'a-ntrig','data':'1'}).get('readback_verified'):raise RuntimeError('network trigger failed')
   if a.exercise_storage_fault:
    wait_network(lambda x:x.get('rx_held_ready',[False,False])[1] is True,'old frame in completed DMA')
   else:
    wait_network(lambda x:x.get('rx_queued',[False,False])[1] is True,'old frame queued')
   if g.call('network_grant_close',{'generation':generation,'boot_id':args['boot_id']})!={'closed':True}:raise RuntimeError('network grant close failed')
   wait_phases('c','c')  # Both guests observed -13 on closure.
   closed=g.call('network_status',{})
   if closed.get('grant_active') is not False or closed.get('tx_queued')!=[False,False] or closed.get('rx_queued')!=[False,False]:raise RuntimeError('network closure did not clear queues: '+str(closed))
   generation2=g.call('network_grant_open',args)['generation']
   if int(generation2)<=int(generation):raise RuntimeError('network grant generation did not advance')
   reject(g,'network_grant_close',{'generation':generation,'boot_id':args['boot_id']},'stale network grant')
   if a.exercise_storage_fault:
    if g.call('debug_network_hold_rx',{'slot':1,'hold':False})!={'held':False}:raise RuntimeError('failed to release held RX')
   wait_network(lambda x:x.get('dropped',[0,0])[1]>=1,'old grant packet dropped')
   if not g.call('file_put',{'name':'a-ntrig','data':'2'}).get('readback_verified'):raise RuntimeError('network second trigger failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'VirtIO network round trips')
   if state.get('incarnations')!=[task_a['incarnation'],task_b['incarnation']] or state.get('outcomes')!=[1,1] or state.get('exit_codes')!=[0,0] or state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('stack_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('network exchange or process cleanup failed: '+str(state))
   network_status=g.call('network_status',{})
   keys={'available','ports','grant_active','grant_generation','grant_slots','grant_incarnations','tx_queued','rx_queued','tx_submitted','tx_completed','rx_accepted','rx_delivered','dropped'}
   if a.exercise_storage_fault:keys.add('rx_held_ready')
   expected_accepted=[2,2] if a.exercise_storage_fault else [2,3]
   if set(network_status)!=keys or network_status.get('grant_active') is not False or network_status.get('tx_queued')!=[False,False] or network_status.get('rx_queued')!=[False,False] or network_status.get('tx_submitted')!=[3,2] or network_status.get('tx_completed')!=[3,2] or network_status.get('rx_accepted')!=expected_accepted or network_status.get('rx_delivered')!=[2,2] or network_status.get('dropped',[0,0])[1]<1:raise RuntimeError('VirtIO queues or grant cleanup invalid: '+str(network_status))
   for name in ('a-nphase','b-nphase','a-ntrig'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('network fixture cleanup failed')
   if g.call('file_list',{})!=files_before:raise RuntimeError('network fixture did not restore file catalogue')
   g.call('upload_reset',{'id':upload_id})
   if a.exercise_storage_fault:
    # Reuse the same two NICs for 1↔2 then 0↔2; no host network.
    for name in ('a-ngate','b-ngate','c-ngate'):
     if any(entry.get('name')==name for entry in files_before):raise RuntimeError('network-three gate already exists')
     if not g.call('file_put',{'name':name,'data':'0'}).get('readback_verified'):raise RuntimeError('network-three setup failed')
    image=bytes.fromhex((Path(__file__).resolve().parent/'fixtures'/'rust-net-three.hex').read_text().strip())
    up=stage_program(g,image)
    na=g.call('process_spawn_staged',{'id':up});nb=g.call('process_spawn_staged',{'id':up});nc=g.call('process_spawn_staged',{'id':up})
    if [na.get('slot'),nb.get('slot'),nc.get('slot')]!=[0,1,2] or len({na.get('boot_id'),nb.get('boot_id'),nc.get('boot_id')})!=1:raise RuntimeError('network-three admission failed')
    def nthree_tasks(predicate,label):
     for _ in range(80):
      snapshot=g.call('process_status',{})
      if predicate(snapshot):return snapshot
      time.sleep(.2)
     raise RuntimeError('network-three tasks did not reach '+label+': '+str(snapshot))
    def ngate(name,value):
     if not g.call('file_put',{'name':name,'data':value}).get('readback_verified'):raise RuntimeError('network-three gate failed')
    def nphase(name,value):
     for _ in range(80):
      result=g.call('file_get',{'name':name}).get('hex')
      if result==value.encode().hex():return
      time.sleep(.2)
     raise RuntimeError('network-three phase not reached: '+str((name,value,result)))
    nthree_tasks(lambda x:x.get('all_slot_states')==['ready']*3 and min(x.get('all_io_completed',[0]*3))>0,'all tasks active')
    bind12={'a_slot':2,'a_incarnation':nc['incarnation_text'],'b_slot':1,'b_incarnation':nb['incarnation_text'],'boot_id':na['boot_id']}
    reject(g,'network_grant_open',dict(bind12,a_slot=3),'invalid network slots')
    prior=g.call('network_status',{})
    old=g.call('network_grant_open',bind12)['generation']
    if g.call('network_status',{}).get('grant_slots')!=[1,2]:raise RuntimeError('network endpoints not sorted')
    for name in ('a-ngate','b-ngate','c-ngate'):ngate(name,'1')
    nphase('a-ngate','v')  # Nonmember 0 observed -13 on send and recv.
    ngate('b-ngate','2');nphase('b-ngate','p');nphase('c-ngate','p')
    wait_network(lambda x:x.get('rx_delivered')==[prior['rx_delivered'][i]+1 for i in range(2)],'1-to-2 real VirtIO exchange')
    if g.call('debug_network_hold_rx',{'slot':1,'hold':True})!={'held':True}:raise RuntimeError('network-three hold failed')
    ngate('b-ngate','3');nphase('b-ngate','s')
    held=wait_network(lambda x:x.get('rx_held_ready',[False,False])[1] is True,'1-to-2 old frame completed in DMA')
    stopped=g.call('process_stop',{'slot':2,'incarnation':nc['incarnation_text'],'boot_id':nc['boot_id']})
    if stopped.get('stopped') is not True:raise RuntimeError('network-three member stop failed')
    revoked=wait_network(lambda x:x.get('grant_active') is False,'network member exit revoked grant')
    if revoked.get('rx_held_ready',[False,False])[1] is not True:raise RuntimeError('old DMA frame not held over revocation')
    ngate('c-ngate','0');nc2=g.call('process_spawn_staged',{'id':up})
    if nc2.get('slot')!=2 or nc2.get('incarnation',0)<=nc['incarnation']:raise RuntimeError('network member readmission failed')
    nthree_tasks(lambda x:x.get('all_slot_states')==['ready']*3,'network member readmitted')
    stale={'a_slot':2,'a_incarnation':nc['incarnation_text'],'b_slot':0,'b_incarnation':na['incarnation_text'],'boot_id':na['boot_id']}
    reject(g,'network_grant_open',stale,'stale process incarnation')
    current=g.call('network_grant_open',dict(stale,a_incarnation=nc2['incarnation_text']))['generation']
    if int(current)<=int(old) or g.call('network_status',{}).get('grant_slots')!=[0,2]:raise RuntimeError('0-to-2 network grant invalid')
    reject(g,'network_grant_close',{'generation':old,'boot_id':na['boot_id']},'stale network grant')
    dropped_before=g.call('network_status',{})['dropped'][1]
    if g.call('debug_network_hold_rx',{'slot':1,'hold':False})!={'held':False}:raise RuntimeError('network-three RX release failed')
    after_drop=wait_network(lambda x:x.get('dropped',[0,0])[1]>dropped_before,'late frame rejected after new grant')
    ngate('b-ngate','7');nphase('b-ngate','v')  # Nonmember 1 refused.
    ngate('a-ngate','4');ngate('c-ngate','4')
    nphase('a-ngate','n');nphase('c-ngate','n')
    wait_network(lambda x:x.get('grant_slots')==[0,2] and x.get('rx_delivered')==[after_drop['rx_delivered'][i]+1 for i in range(2)],'0-to-2 real VirtIO exchange')
    if g.call('network_grant_close',{'generation':current,'boot_id':na['boot_id']})!={'closed':True}:raise RuntimeError('network-three close failed')
    for name in ('a-ngate','b-ngate','c-ngate'):ngate(name,'9')
    done=nthree_tasks(lambda x:x.get('all_slot_states')==['exited']*3,'network-three teardown')
    if done.get('all_outcomes')!=[1]*3 or done.get('all_exit_codes')!=[0]*3 or done.get('all_data_scrubbed')!=[True]*3 or done.get('all_code_scrubbed')!=[True]*3 or done.get('all_stack_scrubbed')!=[True]*3 or done.get('frame_pool_assigned')!=0:raise RuntimeError('network-three cleanup invalid: '+str(done))
    network_final=g.call('network_status',{})
    if network_final.get('grant_active') is not False or network_final.get('grant_slots') is not None or network_final.get('tx_queued')!=[False,False] or network_final.get('rx_queued')!=[False,False]:raise RuntimeError('network-three grant cleanup invalid: '+str(network_final))
    g.call('upload_reset',{'id':up})
    for name in ('a-ngate','b-ngate','c-ngate'):
     if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('network-three gate cleanup failed')
    if g.call('file_list',{})!=files_before:raise RuntimeError('network-three fixture left files')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  if a.exercise_storage_fault:
   v2_fault=Path(tmp)/'program-v2-fault.raw';create(v2_fault,v2_base)
   pf=Guest(a.qemu,guest,v2_fault,tmp,49,out,a.transport)
   try:
    pf.progress('before V2 postcommit failure',timeout_image=True,expected_exit_code=0)
    upload_id=stage_program(pf,linked);before=v2_fault.read_bytes()
    if pf.call('debug_fail_program_v2_postcommit',{})!={'armed':True}:raise RuntimeError('V2 postcommit failpoint not armed')
    reject(pf,'program_store_v2_commit',{'id':upload_id},'injected program v2 postcommit confirmation failure')
    if pf.call('storage_status',{}).get('write_blocked') is not True:raise RuntimeError('uncertain V2 write did not block storage')
    reject(pf,'program_store_v2_commit',{'id':upload_id},'storage write blocked until restart')
    reject(pf,'program_store_commit',{'id':upload_id},'storage write blocked until restart')
    reject(pf,'file_put',{'name':'blocked.txt','data':'x'},'storage write blocked until restart')
    after=v2_fault.read_bytes()
    if before[:64*512]!=after[:64*512] or before[130*512:]!=after[130*512:]:raise RuntimeError('failed V2 write changed outside its region')
    traces.append(pf.trace);states.append(pf.call('storage_status',{}))
   finally:pf.close()
   pf=Guest(a.qemu,guest,v2_fault,tmp,50,out,a.transport)
   try:
    pf.progress('after V2 postcommit restart',timeout_image=True,expected_exit_code=0)
    status=pf.call('program_store_v2_status',{})
    if status.get('generation')!=3 or status.get('write_blocked') is not False:raise RuntimeError('V2 committed record did not reconcile: '+str(status))
    if not pf.call('file_put',{'name':'a-gate','data':'1'}).get('readback_verified'):raise RuntimeError('V2 recovered gate failed')
    pf.wait_boot_identity();task=pf.call('program_store_v2_spawn',{})
    state=pf.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='exited','V2 recovered program exited')
    if state.get('exit_codes',[None,None])[0]!=0 or state.get('stack_scrubbed',[False,False])[0] is not True:raise RuntimeError('V2 recovered program failed: '+str(state))
    traces.append(pf.trace);states.append(pf.call('storage_status',{}))
   finally:pf.close()
   # A postcommit error makes the shared disk write state uncertain.  The
   # restarted VM reconciles this disposable program store from both records.
   program_fault=Path(tmp)/'program-fault.raw';create(program_fault,program_base)
   pf=Guest(a.qemu,guest,program_fault,tmp,39,out,a.transport)
   try:
    pf.progress('before program postcommit failure',timeout_image=True,expected_exit_code=0)
    upload_id=stage_program(pf,rust_elf)
    before=program_fault.read_bytes()
    if pf.call('debug_fail_program_postcommit',{})!={'armed':True}:raise RuntimeError('program postcommit failpoint not armed')
    reject(pf,'program_store_commit',{'id':upload_id},'injected program postcommit confirmation failure')
    if pf.call('storage_status',{}).get('write_blocked') is not True:raise RuntimeError('uncertain program write did not block storage')
    reject(pf,'program_store_commit',{'id':upload_id},'storage write blocked until restart')
    reject(pf,'file_put',{'name':'blocked.txt','data':'x'},'storage write blocked until restart')
    after=program_fault.read_bytes()
    if before[:20*512]!=after[:20*512] or before[40*512:]!=after[40*512:]:raise RuntimeError('failed program write changed outside its region')
    traces.append(pf.trace);states.append(pf.call('storage_status',{}))
   finally:pf.close()
   pf=Guest(a.qemu,guest,program_fault,tmp,40,out,a.transport)
   try:
    pf.progress('after program postcommit restart',timeout_image=True,expected_exit_code=0)
    status=pf.call('program_store_status',{})
    if status.get('generation')!=3 or status.get('write_blocked') is not False:raise RuntimeError('committed program record did not reconcile: '+str(status))
    pf.wait_boot_identity();admitted=pf.call('program_store_spawn',{});wait_rust_program(pf,admitted)
    if pf.call('file_get',{'name':'a-rust'})!={'name':'a-rust','hex':'52'}:raise RuntimeError('recovered program did not execute')
    traces.append(pf.trace);states.append(pf.call('storage_status',{}))
   finally:pf.close()
   # In the same integrated guest, fail confirmation after the commit flush.
   # The kernel must preserve the old in-memory view and deny further writes.
   g=Guest(a.qemu,guest,disk,tmp,10,out,a.transport)
   try:
    for _ in range(10):
     if g.call('ipc_status',{}).get('finished') is True:break
     time.sleep(.2)
    else:raise RuntimeError('fault-injection guest tasks did not finish')
    baseline=g.call('file_put',{'name':'baseline.txt','data':'safe'})
    if not baseline.get('readback_verified'):raise RuntimeError('baseline save failed')
    expected_generation=baseline['generation']+1
    catalog={entry['name']:g.call('file_get',{'name':entry['name']})['hex'] for entry in g.call('file_list',{})}
    if g.call('debug_fail_postcommit',{})!={'armed':True}:raise RuntimeError('injection not armed; build with storage-fault-test')
    reject(g,'file_put',{'name':'uncertain.txt','data':'committed'},'injected postcommit confirmation failure')
    status=g.call('storage_status',{})
    if status.get('write_blocked') is not True:raise RuntimeError('uncertain write was not blocked')
    if g.call('file_get',{'name':'baseline.txt'})!={'name':'baseline.txt','hex':b'safe'.hex()}:raise RuntimeError('baseline read unavailable')
    reject(g,'file_get',{'name':'uncertain.txt'},'file not found')
    reject(g,'file_put',{'name':'later.txt','data':'must-not-write'},'storage write blocked until restart')
    reject(g,'file_delete',{'name':'baseline.txt'},'storage write blocked until restart')
    traces.append(g.trace);states.append(status)
   finally:g.close()
   data=disk.read_bytes();target=1-baseline['slot'];offset=(4 if target==0 else 12)*512
   validate_slot(data[offset:offset+4096],expected_generation,final,{**catalog,'uncertain.txt':b'committed'.hex()})
   g=Guest(a.qemu,guest,disk,tmp,11,out,a.transport)
   try:
    for _ in range(10):
     if g.call('ipc_status',{}).get('finished') is True:break
     time.sleep(.2)
    else:raise RuntimeError('restart guest tasks did not finish')
    status=g.call('storage_status',{})
    if status.get('restore_generation')!=expected_generation or status.get('write_blocked') is not False:raise RuntimeError('restart did not reconcile committed slot')
    if g.call('file_get',{'name':'uncertain.txt'})!={'name':'uncertain.txt','hex':b'committed'.hex()}:raise RuntimeError('committed write lost on restart')
    if not g.call('file_put',{'name':'recovered.txt','data':'ok'}).get('readback_verified'):raise RuntimeError('writes not restored after reboot')
    traces.append(g.trace);states.append(status)
   finally:g.close()
   # A failed initial restore must not permit a later write of the blank
   # in-memory placeholder over either non-blank, invalid on-disk slot.
   damaged_disk=Path(tmp)/'unrestorable.raw';damaged=bytearray(SIZE)
   damaged[4*512]=0x51;damaged[12*512]=0x52;create(damaged_disk,damaged)
   original_hash=sha(damaged_disk.read_bytes())
   g=Guest(a.qemu,guest,damaged_disk,tmp,12,out,a.transport,allow_degraded=True)
   try:
    status=g.call('storage_status',{})
    if status.get('restore')!='invalid' or status.get('restore_slots')!=['invalid','invalid'] or status.get('write_blocked') is not True:raise RuntimeError('invalid restoration not reported and blocked')
    ready=g.call('fixtures_status',{})
    if ready.get('storage_write_blocked') is not True or ready.get('admission_ready') is not False:raise RuntimeError('blocked storage advertised external admission: '+str(ready))
    if g.call('document_snapshot',{})!=blank or g.call('file_list',{})!=[]:raise RuntimeError('unexpected degraded in-memory view')
    reject(g,'file_put',{'name':'unsafe.txt','data':'must-not-write'},'storage write blocked until restart')
    status2=g.call('storage_status',{});task=g.call('el0_status',{})
    if status2.get('write_blocked') is not True or task.get('slice_ticks')!=0:raise RuntimeError('degraded mode did not remain responsive')
    traces.append(g.trace);states.append(status2)
   finally:g.close()
   if sha(damaged_disk.read_bytes())!=original_hash:raise RuntimeError('invalid on-disk snapshot changed')
 if sha(guest.read_bytes())!=h:raise RuntimeError('image changed')
 (out/'workspace-dynamic-storage-evidence.json').write_text(json.dumps({'guest_sha256':h,'qemu_version':qemu,'transport':a.transport,'states':states,'qmp_input':qmp_events,'sessions':traces},indent=2)+'\n')
 print('PASS QEMU EL0 file IPC, fault containment, execution budget, recovery and legacy migration')
 if a.exercise_storage_fault:print('PASS QEMU postcommit uncertainty, write blocking and restart reconciliation')
if __name__=='__main__':main()
