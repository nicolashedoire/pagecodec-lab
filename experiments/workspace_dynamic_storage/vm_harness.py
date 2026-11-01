#!/usr/bin/env python3
"""Test bounded two-slot snapshot recovery using disposable QEMU disks."""
import argparse, hashlib, json, os, shutil, socket, subprocess, sys, tempfile, time
from pathlib import Path
SIZE=1024*1024
OFFSET=4*512
RECORD=4096

def sha(data): return hashlib.sha256(data).hexdigest()
def fnv(data):
    h=2166136261
    for b in data:h=((h^b)*16777619)&0xffffffff
    return h

def create(path,data):
    fd=os.open(path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
    with os.fdopen(fd,'wb') as f:f.write(data)

def connect(path,process):
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        if process.poll() is not None:raise RuntimeError('QEMU exited')
        s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
        try:s.connect(str(path));s.settimeout(15);return s
        except (FileNotFoundError,ConnectionRefusedError):s.close();time.sleep(.1)
    raise TimeoutError('socket timeout')

class Guest:
    def __init__(self,qemu,kernel,image,tmp,index,out,transport,allow_degraded=False,rng=True,network=False):
        self.trace=[];self.qmp_trace=[];self.socket=Path(tmp)/('mcp'+str(index));self.qmp=Path(tmp)/('qmp'+str(index))
        self.disk_image=Path(image);self.output_dir=Path(out);self.run_index=index
        self.log=tempfile.TemporaryFile(mode='w+b')
        f={'driver':'file','filename':str(image),'node-name':'f0'}
        d={'driver':'raw','file':'f0','node-name':'disk0'}
        cmd=[qemu,'-M','virt,virtualization=off','-accel','tcg','-cpu','cortex-a57','-smp','1','-m','128M',
             '-nodefaults','-display','none','-nic','none','-device','virtio-gpu-device','-device','virtio-tablet-device',
             '-chardev',f'socket,id=mcp,path={self.socket},server=on,wait=off','-serial','chardev:mcp',
             '-monitor','none','-qmp',f'unix:{self.qmp},server=on,wait=off','-no-reboot','-kernel',str(kernel),'-global','virtio-mmio.force-legacy='+('true' if transport=='legacy' else 'false'),
             '-blockdev',json.dumps(f),'-blockdev',json.dumps(d),'-device','virtio-blk-device,drive=disk0']
        if rng:cmd+=['-device','virtio-rng-device']
        # Both NICs attach only to QEMU's in-process hub. There is no host
        # backend, user-mode network, bridge, TAP, or external listener.
        if network:
            for i in range(2):
                cmd+=['-netdev',f'hubport,id=internal{i},hubid=0',
                      '-device',f'virtio-net-device,netdev=internal{i},mac=52:54:00:12:34:0{i+1}']
        self.command=cmd
        self.process=subprocess.Popen(cmd,stdout=self.log,stderr=subprocess.STDOUT)
        try:
            self.sock=connect(self.socket,self.process);self.stream=self.sock.makefile('rwb',buffering=0)
            init=self.exchange({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'graph-snapshot-test','version':'0.1'}}})
            if init.get('result',{}).get('protocolVersion')!='2025-11-25':raise RuntimeError('wrong MCP version')
            self.stream.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            self.trace.append({'notification':{'jsonrpc':'2.0','method':'notifications/initialized'}});self.next_id=2
            listed=self.exchange({'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}})
            names={x['name'] for x in listed.get('result',{}).get('tools',[])}
            self.tools=names
            if not {'ipc_status','document_snapshot','document_merge','pointer_state','interaction_state','move_ai','click_ai','mmu_status','el0_status'}.issubset(names):raise RuntimeError('missing MCP tools')
            status=self.call('mmu_status',{})
            expected={'enabled':True,'caches':False,'mapping':'identity','image':'normal-nc','virtio':'device-xn','range_checks':'full','kernel_el0':'samples-denied','user_code':'ro','user_stack':'private-alias-rw','address_spaces':2,'asids':[1,2],'gic':'device-xn'}
            if status!=expected:raise RuntimeError('MMU validation status mismatch: '+str(status))
            preempt=self.call('el0_status',{})
            if not preempt.get('validated') or preempt.get('ticks')!=8 or preempt.get('el')!='EL0' or preempt.get('timer')!='disabled' or preempt.get('interrupts')!='masked':raise RuntimeError('EL0 probe status invalid')
            if preempt.get('private_values')!=[0xa1a1,0xb2b2] or preempt.get('service_asid')!=1:raise RuntimeError('private markers or service ASID invalid')
            states=preempt.get('task_states')
            assigned=preempt.get('frame_pool_assigned')
            # The loaded program may already own its two frames when the
            # status request arrives; teardown is checked later by progress().
            if preempt.get('cross_code_read_denied')!=[True,True] or preempt.get('frame_pool_capacity')!=6 or not isinstance(states,list) or len(states)!=2 or any(s not in ('ready','exited') for s in states) or not isinstance(assigned,int) or not states.count('ready')<=assigned<=4:raise RuntimeError('isolated task code or frame assignment invalid')
            events=preempt.get('events',[]);previous=[0,0]
            if len(events)!=8:raise RuntimeError('EL0 event count')
            for event_index,event in enumerate(events):
                actor=event_index%2;expected_next=1-actor if event_index<7 else 2
                if event.get('task')!=actor or event.get('next')!=expected_next or event.get('count',0)<previous[actor]:raise RuntimeError('EL0 preemption trace invalid: '+str(events))
                previous[actor]=event['count']
            if any(count<=0 for count in previous):raise RuntimeError('EL0 task did not progress across probe: '+str(events))
            counts=preempt.get('slice_counts',[])
            if allow_degraded:
                degraded=self.call('storage_status',{})
                if degraded.get('restore')!='invalid' or degraded.get('write_blocked') is not True or preempt.get('slice_ticks')!=0:raise RuntimeError('degraded startup did not suspend task slices')
            elif preempt.get('mode')!='service-boundary' or preempt.get('slice_ticks',0)<2 or len(counts)!=2 or min(counts)<=0:raise RuntimeError('service slices not active')
            self.last_progress=preempt
            with connect(self.qmp,self.process) as q:
                with q.makefile('rwb',buffering=0) as st:
                    if 'QMP' not in json.loads(st.readline()):raise RuntimeError('bad QMP greeting')
                    screenshot=out/('frame-'+str(index)+'.ppm')
                    for req in [{'execute':'qmp_capabilities'},{'execute':'screendump','arguments':{'filename':str(screenshot)}}]:
                        st.write(json.dumps(req).encode()+b'\n')
                        while True:
                            reply=json.loads(st.readline())
                            if 'error' in reply:raise RuntimeError('QMP: '+str(reply))
                            if 'return' in reply:break
            frame=screenshot.read_bytes()
            if not frame.startswith(b'P6\n800 600\n255\n') or len(set(frame[-100000::997]))<2:raise RuntimeError('graphics not ready')
            self.frame_hash=sha(frame)
        except BaseException:
            if hasattr(self,'stream'): self.stream.close()
            if hasattr(self,'sock'): self.sock.close()
            if self.process.poll() is None:
                self.process.terminate()
                try: self.process.wait(timeout=5)
                except subprocess.TimeoutExpired: self.process.kill(); self.process.wait()
            self.report_log()
            self.log.close()
            raise
    def report_log(self):
        self.log.flush();self.log.seek(0)
        diagnostic=self.log.read().decode('utf-8',errors='replace')
        if diagnostic:print('QEMU diagnostic: '+diagnostic,file=sys.stderr)
    def capture_timeout(self,req,elapsed):
        # Do not retry a timed-out RPC. Attempt to capture the live VM and
        # pause it before copying its disposable disk. Pause may fail; the raw
        # image is diagnostic evidence, not a crash-consistency guarantee.
        prefix=self.output_dir/('timeout-diagnostic-'+str(self.run_index)+'-'+str(time.time_ns()))
        metadata={'index':self.run_index,'pid':self.process.pid,'elapsed_seconds':elapsed,
                  'rpc_method':req.get('method'),'rpc_id':req.get('id'),
                  'tool':req.get('params',{}).get('name'),'request':req,'trace_so_far':self.trace,
                  'command':self.command,'process_exit_code':self.process.poll(),'qmp':{},
                  'pause_confirmed':False,'errors':[]}
        try:
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as q:
                q.settimeout(2);q.connect(str(self.qmp))
                with q.makefile('rwb',buffering=0) as stream:
                    metadata['qmp']['greeting']=json.loads(stream.readline())
                    for name,args in [('qmp_capabilities',None),('query-status',None),('human-monitor-command',{'command-line':'info registers'}),('stop',None)]:
                        command={'execute':name}
                        if args is not None:command['arguments']=args
                        stream.write(json.dumps(command).encode()+b'\n')
                        for _ in range(20):
                            answer=json.loads(stream.readline())
                            if 'event' not in answer:break
                        else:raise RuntimeError('too many QMP events')
                        metadata['qmp'][name]=answer
                        if 'error' in answer:
                            metadata['errors'].append('QMP '+name+': '+repr(answer['error']))
                            if name=='qmp_capabilities':break
                        if name=='stop' and 'return' in answer:metadata['pause_confirmed']=True
        except Exception as error:metadata['errors'].append('QMP: '+repr(error))
        try:
            destination=prefix.with_suffix('.raw');shutil.copyfile(self.disk_image,destination)
            metadata['disk_copy']=str(destination)
        except Exception as error:metadata['errors'].append('disk copy: '+repr(error))
        try:
            self.log.flush();self.log.seek(0);diagnostic=self.log.read().decode('utf-8',errors='replace')
            metadata['qemu_log']=diagnostic
        except Exception as error:metadata['errors'].append('QEMU log: '+repr(error))
        try:
            destination=prefix.with_suffix('.json');destination.write_text(json.dumps(metadata,indent=2)+'\n')
            print('QEMU timeout diagnostic: '+str(destination),file=sys.stderr)
        except Exception as error:print('QEMU timeout diagnostic failed: '+repr(error),file=sys.stderr)
    def exchange(self,req):
        started=time.monotonic()
        try:
            self.stream.write(json.dumps(req,separators=(',',':')).encode()+b'\n')
            raw=self.stream.readline()
        except TimeoutError:
            self.capture_timeout(req,time.monotonic()-started)
            raise
        try:reply=json.loads(raw)
        except ValueError:raise RuntimeError('unexpected guest output: '+repr(raw))
        self.trace.append({'request':req,'response':reply})
        if reply.get('jsonrpc')!='2.0' or reply.get('id')!=req['id'] or 'error' in reply:raise RuntimeError('bad RPC: '+str(reply))
        return reply
    def call(self,tool,args,rejected=False):
        self.next_id+=1
        req={'jsonrpc':'2.0','id':self.next_id,'method':'tools/call','params':{'name':tool,'arguments':args}}
        result=self.exchange(req).get('result',{})
        if result.get('isError') is not rejected:raise RuntimeError('unexpected tool result '+str(result))
        if rejected:return None
        content=result.get('content',[])
        if len(content)!=1 or content[0].get('type')!='text':raise RuntimeError('bad content')
        return json.loads(content[0]['text'])
    def call_text(self,tool,args):
        self.next_id+=1
        result=self.exchange({'jsonrpc':'2.0','id':self.next_id,'method':'tools/call','params':{'name':tool,'arguments':args}}).get('result',{})
        content=result.get('content',[])
        if result.get('isError') is not False or len(content)!=1 or content[0].get('type')!='text':raise RuntimeError('text tool failed')
        return content[0]['text']
    def qmp_command(self,request):
        with connect(self.qmp,self.process) as q:
            with q.makefile('rwb',buffering=0) as st:
                if 'QMP' not in json.loads(st.readline()):raise RuntimeError('bad QMP greeting')
                for req in [{'execute':'qmp_capabilities'},request]:
                    st.write(json.dumps(req).encode()+b'\n')
                    while True:
                        response=json.loads(st.readline())
                        if 'error' in response:raise RuntimeError('QMP error '+str(response))
                        if 'return' in response:break
                if request['execute']=='input-send-event':self.qmp_trace.append({'request':request,'response':response})
                return response
    def progress(self,label,timeout_image=False,expected_exit_code=None):
        before=self.last_progress
        for _ in range(8):
            current=self.call('el0_status',{})
            if current.get('task_states')==['exited','exited'] and current.get('spawn_exited') is True and current.get('image_count')==4 and current.get('images_reclaimed')==[True]*4:break
            time.sleep(.5)
        else:raise RuntimeError('tasks did not finish across '+label)
        if current.get('mode')!='service-boundary' or current.get('slice_ticks',0)<before.get('slice_ticks',0):raise RuntimeError('EL0 slices regressed across '+label)
        if current.get('private_values')!=[0xa1a1,0xb2b2] or current.get('service_asid')!=1:raise RuntimeError('private markers or service ASID changed across '+label)
        if current.get('dispatches',0)<before.get('dispatches',0) or current.get('frame_pool_assigned')!=0 or current.get('frame_scrubbed')!=[True,True] or current.get('alias_removed')!=[True,True]:raise RuntimeError('task teardown or reclamation invalid across '+label)
        if any(current.get(k) is not True for k in ('spawn_admitted','spawn_reused_frame','spawn_exited')):raise RuntimeError('new task did not reuse and release a frame across '+label)
        success=expected_exit_code is None or expected_exit_code==0
        if current.get('spawn_executed') is not success:raise RuntimeError('wrong task exit state across '+label)
        expected=['success','fault','timeout' if timeout_image else 'bad_svc','success' if success else 'exit_error']
        if current.get('image_count')!=4 or current.get('image_outcomes')!=expected or current.get('images_reclaimed')!=[True]*4:raise RuntimeError('fault containment or image cleanup invalid across '+label)
        seen=current.get('image_irq_x19_max_seen')
        if timeout_image and (not isinstance(seen,list) or len(seen)!=4 or seen[2] is not True):raise RuntimeError('timeout did not preempt x19=UINT64_MAX across '+label)
        if current.get('completions_revoked')!=[True]*4 or (not timeout_image and current.get('stale_response_blocked') is not True):raise RuntimeError('IPC response survived task exit across '+label)
        incarnations=current.get('image_incarnations')
        if not isinstance(incarnations,list) or len(incarnations)!=4 or any(not isinstance(x,int) or x<=0 for x in incarnations) or len(set(incarnations))!=4:raise RuntimeError('process incarnations not unique across '+label)
        if not timeout_image and current.get('stale_generation_rejected') is not True:raise RuntimeError('stale completion was not rejected across '+label)
        if current.get('loader_rejected') is not False or any(current.get(k) is not True for k in ('image_loaded','code_alias_removed','code_frame_scrubbed')):raise RuntimeError('program loading or code reclamation invalid across '+label)
        exits=current.get('image_exit_codes')
        if not isinstance(exits,list) or len(exits)!=4 or exits[3]!=expected_exit_code:raise RuntimeError('unexpected loaded program exit code across '+label)
        if len(current.get('slice_counts',[]))!=2 or any(a<b for a,b in zip(current['slice_counts'],before['slice_counts'])):raise RuntimeError('EL0 counts regressed across '+label)
        if any(count >= (1<<63) for count in current['slice_counts']):raise RuntimeError('user register corrupted a kernel counter across '+label)
        again=self.call('el0_status',{})
        if again.get('task_states')!=['exited','exited'] or any(again.get(k)!=current.get(k) for k in ('slice_ticks','dispatches','slice_counts','frame_pool_assigned','frame_scrubbed','alias_removed','spawn_admitted','spawn_reused_frame','spawn_executed','spawn_exited','loader_rejected','image_loaded','code_alias_removed','code_frame_scrubbed','image_count','image_outcomes','images_reclaimed','completions_revoked','stale_response_blocked')):raise RuntimeError('exited task was dispatched again across '+label)
        self.last_progress=current
        return current
    def wait_pair(self,expected_markers=(0xa1a1,0xb2b2)):
        for _ in range(5):
            pair=self.call('process_pair_status',{})
            if pair.get('done') is True:break
            time.sleep(2.4)
        else:raise RuntimeError('two ELF tasks did not finish: '+str(pair))
        identities=pair.get('incarnations')
        if not isinstance(identities,list) or len(identities)!=2 or any(not isinstance(x,int) or x<=0 for x in identities) or identities[0]==identities[1]:raise RuntimeError('pair identities invalid: '+str(pair))
        ticks=pair.get('irq_counts')
        if pair.get('started') is not True or pair.get('mappings_verified') is not True or not isinstance(ticks,list) or len(ticks)!=2 or any(not isinstance(x,int) or x<=0 for x in ticks):raise RuntimeError('pair mappings or preemption invalid: '+str(pair))
        if pair.get('barrier_seen')!=[True,True] or pair.get('live_markers')!=list(expected_markers):raise RuntimeError('pair never held both private markers at the barrier: '+str(pair))
        if pair.get('outcomes')!=[1,1] or pair.get('exit_codes')!=[0,0] or pair.get('markers')!=list(expected_markers) or pair.get('data_scrubbed')!=[True,True] or pair.get('code_scrubbed')!=[True,True] or pair.get('frame_pool_assigned')!=0:raise RuntimeError('pair exit or frame cleanup invalid: '+str(pair))
        return pair
    def wait_single(self):
        for _ in range(5):
            state=self.call('process_status',{})
            if state.get('done') is True:break
            time.sleep(2.4)
        else:raise RuntimeError('single ELF task did not finish: '+str(state))
        identities=state.get('incarnations')
        if state.get('mode')!='single' or state.get('started') is not True or state.get('mappings_verified') is not True or not isinstance(identities,list) or len(identities)!=2 or not isinstance(identities[0],int) or identities[0]<=0 or identities[1]!=0:raise RuntimeError('single task identity or mapping invalid: '+str(state))
        if state.get('outcomes')!=[1,0] or state.get('exit_codes')!=[0,None] or state.get('markers')!=[0xe3e3,0] or state.get('data_scrubbed')!=[True,False] or state.get('code_scrubbed')!=[True,False] or state.get('frame_pool_assigned')!=0:raise RuntimeError('single task outcome or cleanup invalid: '+str(state))
        ticks=state.get('irq_counts')
        if not isinstance(ticks,list) or len(ticks)!=2 or not isinstance(ticks[0],int) or ticks[0]<=0 or ticks[1]!=0:raise RuntimeError('single task preemption invalid: '+str(state))
        if state.get('io_submitted')!=[4,0] or state.get('io_completed')!=[4,0]:raise RuntimeError('single task I/O invalid: '+str(state))
        return state
    def wait_independent(self,predicate,label):
        for _ in range(5):
            state=self.call('process_status',{})
            if state.get('mode')=='independent' and predicate(state):return state
            time.sleep(1.2)
        raise RuntimeError('independent tasks did not reach '+label+': '+str(state))
    def wait_boot_identity(self):
        for _ in range(5):
            status=self.call('process_status',{})
            if status.get('boot_identity_ready') is True:return
            time.sleep(1.2)
        raise RuntimeError('boot entropy did not become available')
    def input(self,events):
        self.qmp_command({'execute':'input-send-event','arguments':{'events':events}})
    def screenshot(self,path):
        self.qmp_command({'execute':'screendump','arguments':{'filename':str(path)}})
        return sha(path.read_bytes())
    def wait_interaction(self,expected):
        for _ in range(5):
            result=self.call('interaction_state',{})
            if result==expected:return
            time.sleep(.4)
        raise RuntimeError('interaction mismatch '+str(result))
    def pointer(self):
        self.next_id+=1
        result=self.exchange({'jsonrpc':'2.0','id':self.next_id,'method':'tools/call','params':{'name':'pointer_state','arguments':{}}}).get('result',{})
        if result.get('isError') is not False or len(result.get('content',[]))!=1:raise RuntimeError('pointer state failed')
        return result['content'][0]['text']
    def close(self):
        failed=sys.exc_info()[0] is not None
        self.stream.close();self.sock.close();self.process.terminate()
        try:self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:self.process.kill();self.process.wait()
        if failed:self.report_log()
        self.log.close()
