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
            if not {'document_edit','document_snapshot','document_save','document_load','document_merge','pointer_state','interaction_state','move_ai','click_ai','mmu_status','el0_status'}.issubset(names):raise RuntimeError('missing MCP tools')
            status=self.call('mmu_status',{})
            expected={'enabled':True,'caches':False,'mapping':'identity','image':'normal-nc','virtio':'device-xn','range_checks':'full','kernel_el0':'samples-denied','user_code':'ro','user_stack':'rw','gic':'device-xn'}
            if status!=expected:raise RuntimeError('MMU validation status mismatch: '+str(status))
            preempt=self.call('el0_status',{})
            if not preempt.get('validated') or preempt.get('ticks')!=8 or preempt.get('el')!='EL0' or preempt.get('timer')!='disabled' or preempt.get('interrupts')!='masked':raise RuntimeError('EL0 probe status invalid')
            events=preempt.get('events',[]);previous=[0,0]
            if len(events)!=8:raise RuntimeError('EL0 event count')
            for index,event in enumerate(events):
                actor=index%2;expected_next=1-actor if index<7 else 2
                if event.get('task')!=actor or event.get('next')!=expected_next or event.get('count',0)<=previous[actor]:raise RuntimeError('EL0 preemption trace invalid: '+str(events))
                previous[actor]=event['count']
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
        reply=json.loads(self.stream.readline());self.trace.append({'request':req,'response':reply})
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

def slot_check(slot, generation, status, expected):
    if slot[:8]!=b'GSLOT001' or int.from_bytes(slot[8:16],'little')!=generation or slot[24]!=status:
        raise RuntimeError('unexpected slot header')
    length=int.from_bytes(slot[16:20],'little')
    if not 0<length<=RECORD-512 or fnv(slot[8:20]+slot[512:512+length])!=int.from_bytes(slot[20:24],'little'):
        raise RuntimeError('slot checksum or length invalid')
    if any(slot[25:512]) or any(slot[512+length:]) or json.loads(slot[512:512+length])!=expected:
        raise RuntimeError('slot padding or snapshot mismatch')


def edit(guest,actor,key,value):
    guest.call('document_edit',{'actor':actor,'space':actor+'-space','key':key,'value':value})


def expect_load(guest,slot,generation,snapshot):
    loaded=guest.call('document_load',{})
    if loaded!={'operation':'load','slot':slot,'generation':generation}:raise RuntimeError('wrong recovery selection')
    if guest.call('document_snapshot',{})!=snapshot:raise RuntimeError('wrong recovered snapshot')
    merge=guest.call('document_merge',{});fields={x['key']:x for x in merge['fields']}
    title=snapshot['human']['title']
    if merge['conflicts']!=[{'key':'title','base':'base','human':title,'assistant':'assistant-version'}] or merge['materializable'] is not False or fields['alpha']['origin']!='human' or fields['beta']['origin']!='assistant':
        raise RuntimeError('recovered conflict/origins differ')
    return merge


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--output-dir',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern')
    args=p.parse_args();guest=Path(args.guest);guest_hash=sha(guest.read_bytes());out=Path(args.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('output directory must be empty')
    version=subprocess.run([args.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
    v1={'base':{'title':'base','common':'seed'},'human':{'title':'human-version','alpha':'human-only','common':'seed'},'assistant':{'title':'assistant-version','beta':'assistant-only','common':'seed'}}
    v2={'base':v1['base'],'human':{'title':'human-v2','alpha':'human-only','common':'seed'},'assistant':v1['assistant']}
    traces=[];frames=[]
    with tempfile.TemporaryDirectory(prefix='graph-recovery-') as tmp:
        disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE))
        first=Guest(args.qemu,guest,disk,tmp,1,out,args.transport)
        try:
            baseline={'human':{'focus':'none','clicks':0,'pressed':False},'assistant':{'focus':'none','clicks':0}}
            if first.call('interaction_state',{})!=baseline:raise RuntimeError('initial interaction')
            first.input([{'type':'abs','data':{'axis':'x','value':8500}},{'type':'abs','data':{'axis':'y','value':24575}}])
            for _ in range(5):
                if first.pointer()=='human=114,444; ai=560,330':break
                time.sleep(.4)
            else:raise RuntimeError('human pointer missed button')
            human_before=first.screenshot(out/'human-before.ppm')
            first.input([{'type':'btn','data':{'button':'left','down':True}}])
            down={'human':{'focus':'action','clicks':1,'pressed':True},'assistant':{'focus':'none','clicks':0}}
            first.wait_interaction(down);human_after=first.screenshot(out/'human-after.ppm')
            if human_before==human_after:raise RuntimeError('human click did not change framebuffer')
            first.input([{'type':'btn','data':{'button':'left','down':False}}])
            released={'human':{'focus':'action','clicks':1,'pressed':False},'assistant':{'focus':'none','clicks':0}}
            first.wait_interaction(released)
            if first.call_text('move_ai',{'x':660,'y':444})!='moved AI pointer to 660,444':raise RuntimeError('assistant pointer move')
            ai_before=first.screenshot(out/'assistant-before.ppm')
            if first.call('click_ai',{})!={'focus':'select','clicks':1}:raise RuntimeError('assistant click')
            interacted={'human':{'focus':'action','clicks':1,'pressed':False},'assistant':{'focus':'select','clicks':1}}
            if first.call('interaction_state',{})!=interacted:raise RuntimeError('actor focus interference')
            ai_after=first.screenshot(out/'assistant-after.ppm')
            if ai_before==ai_after:raise RuntimeError('assistant click did not change framebuffer')
            qmp_input=first.qmp_trace
            interaction_frames={'human_before':human_before,'human_after':human_after,'assistant_before':ai_before,'assistant_after':ai_after}
            for actor,key,value in [('human','title','human-version'),('human','alpha','human-only'),('assistant','title','assistant-version'),('assistant','beta','assistant-only')]:edit(first,actor,key,value)
            if first.call('document_snapshot',{})!=v1:raise RuntimeError('initial state mismatch')
            if first.call('document_save',{'mode':'commit'})!={'operation':'commit','slot':0,'generation':1}:raise RuntimeError('first commit')
            if first.call('interaction_state',{})!=interacted or first.pointer()!='human=114,444; ai=660,444':raise RuntimeError('save changed pointer or focus state')
            traces.append(first.trace);frames.append(first.frame_hash)
        finally:first.close()
        after_first=disk.read_bytes();s0=after_first[OFFSET:OFFSET+RECORD];slot_check(s0,1,0xa5,v1)
        if after_first[:OFFSET]!=bytes(OFFSET) or after_first[OFFSET+RECORD:]!=bytes(SIZE-OFFSET-RECORD):raise RuntimeError('first commit touched outside slot')
        second=Guest(args.qemu,guest,disk,tmp,2,out,args.transport)
        try:
            if second.call('interaction_state',{})!=baseline:raise RuntimeError('unexpected persisted focus')
            pointer_before=second.pointer()
            expect_load(second,0,1,v1)
            if second.call('interaction_state',{})!=baseline or second.pointer()!=pointer_before:raise RuntimeError('load changed pointer or focus state')
            edit(second,'human','title','human-v2')
            if second.call('document_snapshot',{})!=v2:raise RuntimeError('second state mismatch')
            if second.call('document_save',{'mode':'stage'})!={'operation':'stage','slot':1,'generation':2}:raise RuntimeError('stage failpoint')
            traces.append(second.trace);frames.append(second.frame_hash)
        finally:second.close() # Host terminates QEMU before commit; no physical power-loss claim.
        after_stage=disk.read_bytes();s1=after_stage[OFFSET+RECORD:OFFSET+2*RECORD]
        if after_stage[OFFSET:OFFSET+RECORD]!=s0 or after_stage[:OFFSET]!=after_first[:OFFSET] or after_stage[OFFSET+2*RECORD:]!=after_first[OFFSET+2*RECORD:]:raise RuntimeError('stage modified prior slot or unrelated sectors')
        slot_check(s1,2,0,v2)
        third=Guest(args.qemu,guest,disk,tmp,3,out,args.transport)
        try:
            expect_load(third,0,1,v1);edit(third,'human','title','human-v2')
            if third.call('document_save',{'mode':'commit'})!={'operation':'commit','slot':1,'generation':2}:raise RuntimeError('second commit')
            traces.append(third.trace);frames.append(third.frame_hash)
        finally:third.close()
        committed=disk.read_bytes();s1=committed[OFFSET+RECORD:OFFSET+2*RECORD];slot_check(s1,2,0xa5,v2)
        if committed[OFFSET:OFFSET+RECORD]!=s0 or committed[:OFFSET]!=bytes(OFFSET) or committed[OFFSET+2*RECORD:]!=bytes(SIZE-OFFSET-2*RECORD):raise RuntimeError('unexpected writes')
        fourth=Guest(args.qemu,guest,disk,tmp,4,out,args.transport)
        try:
            expect_load(fourth,1,2,v2);traces.append(fourth.trace);frames.append(fourth.frame_hash)
        finally:fourth.close()
        if disk.read_bytes()!=committed:raise RuntimeError('load changed disk')
        damaged=bytearray(committed);damaged[OFFSET+RECORD+512]^=1
        bad=Path(tmp)/'corrupt-newest.raw';create(bad,damaged)
        fifth=Guest(args.qemu,guest,bad,tmp,5,out,args.transport)
        try:
            expect_load(fifth,0,1,v1);traces.append(fifth.trace);frames.append(fifth.frame_hash)
        finally:fifth.close()
        if bad.read_bytes()!=damaged:raise RuntimeError('fallback changed corrupted image')
        damaged[OFFSET+512]^=1
        both=Path(tmp)/'corrupt-both.raw';create(both,damaged)
        sixth=Guest(args.qemu,guest,both,tmp,6,out,args.transport)
        try:
            baseline=sixth.call('document_snapshot',{});sixth.call('document_load',{},rejected=True)
            if sixth.call('document_snapshot',{})!=baseline:raise RuntimeError('invalid recovery changed active snapshot')
            traces.append(sixth.trace);frames.append(sixth.frame_hash)
        finally:sixth.close()
        if both.read_bytes()!=damaged:raise RuntimeError('failed recovery changed disk')
        # An interrupted first-ever write must not be silently treated as a
        # new empty store or overwritten by a subsequent save.
        first_stage_disk=Path(tmp)/'first-stage.raw';create(first_stage_disk,bytes(SIZE))
        seventh=Guest(args.qemu,guest,first_stage_disk,tmp,7,out,args.transport)
        try:
            fresh=seventh.call('document_snapshot',{})
            if seventh.call('document_save',{'mode':'stage'})!={'operation':'stage','slot':0,'generation':1}:raise RuntimeError('first stage failed')
            traces.append(seventh.trace);frames.append(seventh.frame_hash)
        finally:seventh.close()
        initial_staged=first_stage_disk.read_bytes();slot_check(initial_staged[OFFSET:OFFSET+RECORD],1,0,fresh)
        eighth=Guest(args.qemu,guest,first_stage_disk,tmp,8,out,args.transport)
        try:
            baseline=eighth.call('document_snapshot',{})
            eighth.call('document_load',{},rejected=True)
            eighth.call('document_save',{'mode':'commit'},rejected=True)
            if eighth.call('document_snapshot',{})!=baseline:raise RuntimeError('failed first recovery changed active snapshot')
            traces.append(eighth.trace);frames.append(eighth.frame_hash)
        finally:eighth.close()
        if first_stage_disk.read_bytes()!=initial_staged:raise RuntimeError('failed first recovery overwrote disk')
    if sha(guest.read_bytes())!=guest_hash:raise RuntimeError('guest changed during test')
    evidence={'guest_sha256':guest_hash,'qemu_version':version,'transport':args.transport,'disk':'exclusively created synthetic 1 MiB image',
              'first_committed_slot_sha256':sha(s0),'staged_slot_sha256':sha(after_stage[OFFSET+RECORD:OFFSET+2*RECORD]),'second_committed_slot_sha256':sha(s1),
              'v1':v1,'v2':v2,'qmp_input':qmp_input,'interaction_frame_hashes':interaction_frames,'sessions':traces,'framebuffers':[{'file':f'frame-{i}.ppm','sha256':h} for i,h in enumerate(frames,1)],
              'limitations':'synthetic QMP input; host terminates QEMU after stage response; no real power loss, atomicity, guaranteed flush, authentication or process isolation'}
    (out/'system-vm-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    print('PASS EL0 preemption, MMU-backed interactive panels and two-slot recovery in QEMU')

if __name__=='__main__':main()
