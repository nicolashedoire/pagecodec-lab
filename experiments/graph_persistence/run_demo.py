#!/usr/bin/env python3
"""Run synthetic graphical MCP snapshot save/restore using only disposable QEMU disks."""
import argparse, hashlib, json, os, socket, subprocess, tempfile, time
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
        self.trace=[];self.socket=Path(tmp)/('mcp'+str(index));self.qmp=Path(tmp)/('qmp'+str(index))
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
            if not {'document_edit','document_snapshot','document_save','document_load','document_merge','pointer_state'}.issubset(names):raise RuntimeError('missing MCP tools')
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
            self.log.close()
            raise
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
    def pointer(self):
        self.next_id+=1
        result=self.exchange({'jsonrpc':'2.0','id':self.next_id,'method':'tools/call','params':{'name':'pointer_state','arguments':{}}}).get('result',{})
        if result.get('isError') is not False or len(result.get('content',[]))!=1:raise RuntimeError('pointer state failed')
        return result['content'][0]['text']
    def close(self):
        self.stream.close();self.sock.close();self.process.terminate()
        try:self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:self.process.kill();self.process.wait()
        self.log.close()

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--qemu',required=True);p.add_argument('--guest',required=True);p.add_argument('--output-dir',required=True);p.add_argument('--transport',choices=['modern','legacy'],default='modern')
    args=p.parse_args();guest=Path(args.guest);guest_hash=sha(guest.read_bytes());out=Path(args.output_dir).resolve();out.mkdir(parents=True,exist_ok=True)
    version=subprocess.run([args.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
    expected={'base':{'title':'base','common':'seed'},'human':{'title':'human-version','alpha':'human-only'},'assistant':{'title':'assistant-version','common':'seed','beta':'assistant-only'}}
    with tempfile.TemporaryDirectory(prefix='graph-snapshot-') as tmp:
        disk=Path(tmp)/'disk.raw';create(disk,bytes(SIZE))
        first=Guest(args.qemu,guest,disk,tmp,1,out,args.transport)
        try:
            before=first.call('document_snapshot',{})
            for actor,space,key,value in [('human','human-space','title','human-version'),('human','human-space','alpha','human-only'),('human','human-space','common',None),('assistant','assistant-space','title','assistant-version'),('assistant','assistant-space','beta','assistant-only')]:
                first.call('document_edit',{'actor':actor,'space':space,'key':key,'value':value})
            if first.call('document_snapshot',{})!=expected:raise RuntimeError('bad edited snapshot')
            merged=first.call('document_merge',{});fields={x['key']:x for x in merged['fields']}
            if merged['conflicts']!=[{'key':'title','base':'base','human':'human-version','assistant':'assistant-version'}] or merged['materializable'] is not False or fields['alpha']['origin']!='human' or fields['beta']['origin']!='assistant' or fields['common']!={'key':'common','value':None,'origin':'human'}:raise RuntimeError('bad merge')
            if first.call('document_save',{})!={'operation':'save','bytes':RECORD,'sector':4}:raise RuntimeError('bad save')
            trace1,frame1=first.trace,first.frame_hash
        finally:first.close()
        data=disk.read_bytes();record=data[OFFSET:OFFSET+RECORD]
        if data[:OFFSET]!=bytes(OFFSET) or data[OFFSET+RECORD:]!=bytes(SIZE-OFFSET-RECORD) or record[:8]!=b'GRAPHS01':raise RuntimeError('unexpected disk writes')
        length=int.from_bytes(record[8:12],'little');payload=record[16:16+length]
        if not 0<length<=RECORD-16 or fnv(payload)!=int.from_bytes(record[12:16],'little') or any(record[16+length:]) or json.loads(payload)!=expected:raise RuntimeError('bad saved record')
        second=Guest(args.qemu,guest,disk,tmp,2,out,args.transport)
        try:
            if second.call('document_snapshot',{})!=before:raise RuntimeError('bad fresh state')
            pointer_before=second.pointer()
            second.call('document_load',{})
            if second.call('document_snapshot',{})!=expected or second.call('document_merge',{})!=merged:raise RuntimeError('restore mismatch')
            if second.pointer()!=pointer_before:raise RuntimeError('restore changed pointer state')
            trace2,frame2=second.trace,second.frame_hash
        finally:second.close()
        if disk.read_bytes()!=data:raise RuntimeError('load modified disk')
        corrupt=bytearray(data);corrupt[OFFSET+16]^=1;bad=Path(tmp)/'bad.raw';create(bad,corrupt)
        third=Guest(args.qemu,guest,bad,tmp,3,out,args.transport)
        try:
            baseline=third.call('document_snapshot',{});third.call('document_load',{},rejected=True)
            if third.call('document_snapshot',{})!=baseline:raise RuntimeError('corrupt load changed state')
            trace3,frame3=third.trace,third.frame_hash
        finally:third.close()
        if bad.read_bytes()!=corrupt:raise RuntimeError('corrupt load changed disk')
        # A valid-checksum payload with an incomplete snapshot tests validation
        # independently from checksum rejection.
        invalid=bytearray(data);bad_payload=b'{"base":{},"human":{}}'
        invalid[OFFSET+8:OFFSET+12]=len(bad_payload).to_bytes(4,'little')
        invalid[OFFSET+12:OFFSET+16]=fnv(bad_payload).to_bytes(4,'little')
        invalid[OFFSET+16:OFFSET+RECORD]=bytes(RECORD-16)
        invalid[OFFSET+16:OFFSET+16+len(bad_payload)]=bad_payload
        shape=Path(tmp)/'shape.raw';create(shape,invalid)
        fourth=Guest(args.qemu,guest,shape,tmp,4,out,args.transport)
        try:
            baseline=fourth.call('document_snapshot',{});fourth.call('document_load',{},rejected=True)
            if fourth.call('document_snapshot',{})!=baseline:raise RuntimeError('invalid shape changed state')
            trace4,frame4=fourth.trace,fourth.frame_hash
        finally:fourth.close()
        if shape.read_bytes()!=invalid:raise RuntimeError('invalid shape changed disk')
        too_long=bytearray(data);too_long[OFFSET+8:OFFSET+12]=(RECORD).to_bytes(4,'little')
        length_disk=Path(tmp)/'length.raw';create(length_disk,too_long)
        fifth=Guest(args.qemu,guest,length_disk,tmp,5,out,args.transport)
        try:
            baseline=fifth.call('document_snapshot',{});fifth.call('document_load',{},rejected=True)
            if fifth.call('document_snapshot',{})!=baseline:raise RuntimeError('invalid length changed state')
            trace5,frame5=fifth.trace,fifth.frame_hash
        finally:fifth.close()
        if length_disk.read_bytes()!=too_long:raise RuntimeError('invalid length changed disk')
    if sha(guest.read_bytes())!=guest_hash:raise RuntimeError('guest changed during test')
    evidence={'guest_sha256':guest_hash,'qemu_version':version,'transport':args.transport,'disk':'exclusive disposable 1 MiB synthetic image','record_sha256':sha(record),'snapshot':expected,'merge':merged,
              'save_session':trace1,'restore_session':trace2,'corrupt_session':trace3,'invalid_shape_session':trace4,'invalid_length_session':trace5,'framebuffers':[{'file':f'frame-{i}.ppm','sha256':h} for i,h in enumerate((frame1,frame2,frame3,frame4,frame5),1)],
              'limitations':'five QEMU instances; no authentication, crash durability, transactions or physical devices'}
    (out/'graph-persistence-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    print('PASS graphical MCP snapshot save, second-VM restore and corrupt-load rejection')

if __name__=='__main__':main()
