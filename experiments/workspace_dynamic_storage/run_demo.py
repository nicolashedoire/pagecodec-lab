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

def independent_elf(reference_hex,code):
 # Reuse only the bounded ELF header; instructions are supplied by the
 # launcher, persisted in VFS, and not linked into the kernel image.
 reference=bytes.fromhex(reference_hex)
 if len(reference)<120 or reference[:4]!=b'\x7fELF' or not code or len(code)%4 or len(code)>392:raise RuntimeError('invalid external program template')
 elf=bytearray(reference[:120]);elf[96:104]=len(code).to_bytes(8,'little');elf[104:112]=len(code).to_bytes(8,'little')
 return (elf+code).hex()

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

def expected_list(catalog):
 return [{'name':name,'size':len(hexbytes)//2} for name,hexbytes in sorted(catalog.items())]

def ipc_history_matches(ipc,base,consumed):
 accepted=ipc.get('accepted');applied=ipc.get('applied')
 return accepted in (base,base+[6],base+[6,6]) and applied in (base,base+[6],base+[6,6]) and len(applied)<=len(accepted) and ipc.get('consumed') in (consumed,consumed+[6])

def main():
 p=argparse.ArgumentParser();p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern');p.add_argument('--output-dir',required=True);p.add_argument('--exercise-storage-fault',action='store_true',help='require a guest built with storage-fault-test');a=p.parse_args()
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
   for _ in range(60):
    fixture=g.call('fixtures_status',{})
    if fixture.get('admission_ready') is True:break
    time.sleep(.2)
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
   for _ in range(60):
    fixture=g.call('fixtures_status',{})
    if fixture.get('admission_ready') is True:break
    time.sleep(.2)
   else:raise RuntimeError('independent startup pause did not become idle: '+str(fixture))
   if fixture.get('pause_requested') is not True or fixture.get('image_count')!=0:raise RuntimeError('independent scenario did not pause fixtures')
   task_a=g.call('process_spawn',{'name':'a-ind.elf'})
   if task_a.get('started') is not True or task_a.get('slot')!=0 or not isinstance(task_a.get('incarnation'),int) or task_a.get('incarnation_text')!=str(task_a['incarnation']):raise RuntimeError('first independent admission failed: '+str(task_a))
   state=g.wait_independent(lambda s:s.get('slot_states',[None,None])[0]=='ready' and s.get('io_completed',[0,0])[0]>0,'A live and reading')
   if state.get('incarnations',[None,None])[0]!=task_a['incarnation']:raise RuntimeError('A identity changed before B admission')
   b=g.call('process_spawn',{'name':'b-ind.elf'})
   if b.get('started') is not True or b.get('slot')!=1 or not isinstance(b.get('incarnation'),int) or b['incarnation']<=task_a['incarnation']:raise RuntimeError('second independent admission failed: '+str(b))
   state=g.wait_independent(lambda s:s.get('slot_states')==['ready','ready'] and min(s.get('io_completed',[0,0]))>0,'A and B live and reading')
   if state.get('incarnations')!=[task_a['incarnation'],b['incarnation']] or state.get('frame_pool_assigned')!=4:raise RuntimeError('independent tasks not isolated concurrently: '+str(state))
   b_reads=state['io_completed'][1]
   reject(g,'process_spawn',{'name':'a-ind.elf'},'no free process slot')
   reject(g,'process_stop',{'slot':0,'incarnation':task_a['incarnation']},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':0,'incarnation':'0'+task_a['incarnation_text']},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':0,'incarnation':'18446744073709551616'},'invalid process_stop arguments')
   reject(g,'process_stop',{'slot':2,'incarnation':task_a['incarnation_text']},'invalid process slot')
   stopped=g.call('process_stop',{'slot':0,'incarnation':task_a['incarnation_text']})
   if stopped!={'stopped':True,'slot':0,'incarnation':task_a['incarnation_text']}:raise RuntimeError('targeted stop failed: '+str(stopped))
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','ready'] and s.get('io_completed',[0,0])[1]>b_reads,'A exited while B continued reading')
   if state.get('incarnations')!=[task_a['incarnation'],b['incarnation']] or state.get('outcomes',[None,None])[0]!=4 or state.get('exit_codes',[0,0])[0] is not None or state.get('data_scrubbed',[False,False])[0] is not True or state.get('code_scrubbed',[False,False])[0] is not True or state.get('frame_pool_assigned')!=2:raise RuntimeError('A teardown disturbed B: '+str(state))
   b_reads=state['io_completed'][1]
   if not g.call('file_put',{'name':'a-gate','data':'0'}).get('readback_verified'):raise RuntimeError('A gate reset failed')
   a2=g.call('process_spawn',{'name':'a-ind.elf'})
   if a2.get('started') is not True or a2.get('slot')!=0 or not isinstance(a2.get('incarnation'),int) or a2.get('incarnation_text')!=str(a2['incarnation']) or a2['incarnation']<=b['incarnation']:raise RuntimeError('A readmission invalid: '+str(a2))
   reject(g,'process_stop',{'slot':0,'incarnation':task_a['incarnation_text']},'stale process incarnation')
   state=g.wait_independent(lambda s:s.get('slot_states')==['ready','ready'] and s.get('io_completed',[0,0])[0]>0 and s.get('io_completed',[0,0])[1]>b_reads,'A readmitted while B continued reading')
   if state.get('incarnations')!=[a2['incarnation'],b['incarnation']] or state.get('frame_pool_assigned')!=4:raise RuntimeError('A readmission disturbed B: '+str(state))
   if not g.call('file_put',{'name':'a-gate','data':'1'}).get('readback_verified'):raise RuntimeError('A second release failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','ready'],'readmitted A exited while B live')
   if state.get('incarnations')!=[a2['incarnation'],b['incarnation']] or state.get('exit_codes',[None,None])[0]!=0 or state.get('frame_pool_assigned')!=2:raise RuntimeError('second A teardown disturbed B: '+str(state))
   if not g.call('file_put',{'name':'b-gate','data':'1'}).get('readback_verified'):raise RuntimeError('B gate release failed')
   state=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'both tasks exited')
   if state.get('exit_codes')!=[0,0] or state.get('data_scrubbed')!=[True,True] or state.get('code_scrubbed')!=[True,True] or state.get('frame_pool_assigned')!=0:raise RuntimeError('independent task cleanup failed: '+str(state))
   for name in ('a-gate','b-gate','a-ind.elf','b-ind.elf','fixture.pause'):
    if not g.call('file_delete',{'name':name}).get('readback_verified'):raise RuntimeError('independent fixture cleanup failed')
   if g.call('fixtures_resume',{}).get('pause_requested') is not False:raise RuntimeError('independent fixture resume failed')
   after=g.progress('fixtures resumed after independent processes',timeout_image=True,expected_exit_code=0)
   if after.get('private_values')!=[0xa1a1,0xb2b2]:raise RuntimeError('independent mode changed historical fixture status')
   traces.append(g.trace);states.append(g.call('storage_status',{}))
  finally:g.close()
  if a.exercise_storage_fault:
   # In the same integrated guest, fail confirmation after the commit flush.
   # The kernel must preserve the old in-memory view and deny further writes.
   g=Guest(a.qemu,guest,disk,tmp,10,out,a.transport)
   try:
    for _ in range(20):
     if g.call('ipc_status',{}).get('finished') is True:break
     time.sleep(.1)
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
    for _ in range(20):
     if g.call('ipc_status',{}).get('finished') is True:break
     time.sleep(.1)
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
