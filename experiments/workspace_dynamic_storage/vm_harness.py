#!/usr/bin/env python3
"""Test bounded two-slot snapshot recovery using disposable QEMU disks."""
import argparse, hashlib, json, os, socket, subprocess, sys, tempfile, time
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
    def __init__(self,qemu,kernel,image,tmp,index,out,transport):
        self.trace=[];self.qmp_trace=[];self.socket=Path(tmp)/('mcp'+str(index));self.qmp=Path(tmp)/('qmp'+str(index))
        self.log=tempfile.TemporaryFile(mode='w+b')
        f={'driver':'file','filename':str(image),'node-name':'f0'}
        d={'driver':'raw','file':'f0','node-name':'disk0'}
        cmd=[qemu,'-M','virt,virtualization=off','-accel','tcg','-cpu','cortex-a57','-smp','1','-m','128M',
             '-nodefaults','-display','none','-nic','none','-device','virtio-gpu-device','-device','virtio-tablet-device',
             '-chardev',f'socket,id=mcp,path={self.socket},server=on,wait=off','-serial','chardev:mcp',
             '-monitor','none','-qmp',f'unix:{self.qmp},server=on,wait=off','-no-reboot','-kernel',str(kernel),'-global','virtio-mmio.force-legacy='+('true' if transport=='legacy' else 'false'),
             '-blockdev',json.dumps(f),'-blockdev',json.dumps(d),'-device','virtio-blk-device,drive=disk0']
        self.process=subprocess.Popen(cmd,stdout=self.log,stderr=subprocess.STDOUT)
        try:
            self.sock=connect(self.socket,self.process);self.stream=self.sock.makefile('rwb',buffering=0)
            init=self.exchange({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'graph-snapshot-test','version':'0.1'}}})
            if init.get('result',{}).get('protocolVersion')!='2025-11-25':raise RuntimeError('wrong MCP version')
            self.stream.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            self.trace.append({'notification':{'jsonrpc':'2.0','method':'notifications/initialized'}});self.next_id=2
            listed=self.exchange({'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}})
            names={x['name'] for x in listed.get('result',{}).get('tools',[])}
            if not {'ipc_status','document_snapshot','document_merge','pointer_state','interaction_state','move_ai','click_ai','mmu_status','el0_status'}.issubset(names):raise RuntimeError('missing MCP tools')
            status=self.call('mmu_status',{})
            expected={'enabled':True,'caches':False,'mapping':'identity','image':'normal-nc','virtio':'device-xn','range_checks':'full','kernel_el0':'samples-denied','user_code':'ro','user_stack':'private-alias-rw','address_spaces':2,'asids':[1,2],'gic':'device-xn'}
            if status!=expected:raise RuntimeError('MMU validation status mismatch: '+str(status))
            preempt=self.call('el0_status',{})
            if not preempt.get('validated') or preempt.get('ticks')!=8 or preempt.get('el')!='EL0' or preempt.get('timer')!='disabled' or preempt.get('interrupts')!='masked':raise RuntimeError('EL0 probe status invalid')
            if preempt.get('private_values')!=[0xa1a1,0xb2b2] or preempt.get('service_asid')!=1:raise RuntimeError('private markers or service ASID invalid')
            states=preempt.get('task_states')
            if preempt.get('cross_code_read_denied')!=[True,True] or preempt.get('frame_pool_capacity')!=4 or not isinstance(states,list) or len(states)!=2 or any(s not in ('ready','exited') for s in states) or preempt.get('frame_pool_assigned')!=states.count('ready'):raise RuntimeError('isolated task code or frame assignment invalid')
            events=preempt.get('events',[]);previous=[0,0]
            if len(events)!=8:raise RuntimeError('EL0 event count')
            for event_index,event in enumerate(events):
                actor=event_index%2;expected_next=1-actor if event_index<7 else 2
                if event.get('task')!=actor or event.get('next')!=expected_next or event.get('count',0)<=previous[actor]:raise RuntimeError('EL0 preemption trace invalid: '+str(events))
                previous[actor]=event['count']
            counts=preempt.get('slice_counts',[])
            if preempt.get('mode')!='service-boundary' or preempt.get('slice_ticks',0)<2 or len(counts)!=2 or min(counts)<=0:raise RuntimeError('service slices not active')
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
    def exchange(self,req):
        self.stream.write(json.dumps(req,separators=(',',':')).encode()+b'\n')
        raw=self.stream.readline()
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
    def progress(self,label):
        before=self.last_progress
        for _ in range(8):
            current=self.call('el0_status',{})
            if current.get('task_states')==['exited','exited'] and current.get('spawn_exited') is True:break
            time.sleep(.1)
        else:raise RuntimeError('tasks did not finish across '+label)
        if current.get('mode')!='service-boundary' or current.get('slice_ticks',0)<before.get('slice_ticks',0):raise RuntimeError('EL0 slices regressed across '+label)
        if current.get('private_values')!=[0xa1a1,0xb2b2] or current.get('service_asid')!=1:raise RuntimeError('private markers or service ASID changed across '+label)
        if current.get('dispatches',0)<before.get('dispatches',0) or current.get('frame_pool_assigned')!=0 or current.get('frame_scrubbed')!=[True,True] or current.get('alias_removed')!=[True,True]:raise RuntimeError('task teardown or reclamation invalid across '+label)
        if any(current.get(k) is not True for k in ('spawn_admitted','spawn_reused_frame','spawn_executed','spawn_exited')):raise RuntimeError('new task did not reuse and release a frame across '+label)
        if current.get('loader_rejected') is not False or any(current.get(k) is not True for k in ('image_loaded','code_alias_removed','code_frame_scrubbed')):raise RuntimeError('program loading or code reclamation invalid across '+label)
        if len(current.get('slice_counts',[]))!=2 or any(a<b for a,b in zip(current['slice_counts'],before['slice_counts'])):raise RuntimeError('EL0 counts regressed across '+label)
        again=self.call('el0_status',{})
        if again.get('task_states')!=['exited','exited'] or any(again.get(k)!=current.get(k) for k in ('slice_ticks','dispatches','slice_counts','frame_pool_assigned','frame_scrubbed','alias_removed','spawn_admitted','spawn_reused_frame','spawn_executed','spawn_exited','loader_rejected','image_loaded','code_alias_removed','code_frame_scrubbed')):raise RuntimeError('exited task was dispatched again across '+label)
        self.last_progress=current
        return current
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
