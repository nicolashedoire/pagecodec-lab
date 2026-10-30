#!/usr/bin/env python3
"""Launch the isolated QEMU guest, exercise MCP and save real VM evidence.

This is a host-side transport harness; the guest code executes only inside QEMU.
"""
import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def connect(path, process):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("QEMU exited before its socket became available")
        stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            stream.connect(str(path))
            stream.settimeout(15)
            return stream
        except (FileNotFoundError, ConnectionRefusedError):
            stream.close()
            time.sleep(0.1)
    raise TimeoutError("QEMU socket did not become available")


def qmp_command(stream, request):
    stream.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
    while True:
        line = stream.readline()
        if not line:
            raise RuntimeError("QMP disconnected")
        response = json.loads(line)
        if "return" in response:
            return response
        if "error" in response:
            raise RuntimeError("QMP failed: " + str(response["error"]))


def wait_for_guest_frame(stream, filename, process):
    """Wait for the guest's 800x600 non-uniform display, not the QEMU splash."""
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("QEMU exited while waiting for guest display")
        try:
            qmp_command(stream, {"execute": "screendump", "arguments": {"filename": str(filename)}})
            data = filename.read_bytes()
            # PPM P6 as written by QEMU: header and raw RGB bytes. A small sample
            # detects a drawn surface rather than a single-colour startup frame.
            parts = data.split(b"\n", 3)
            if (len(parts) == 4 and parts[0] == b"P6" and parts[1] == b"800 600"
                    and parts[2] == b"255" and len(set(parts[3][::997])) > 1):
                return
        except (OSError, RuntimeError):
            # The display may not be initialized yet. Retry until the deadline.
            pass
        time.sleep(0.3)
    raise TimeoutError("guest did not display an initialized 800x600 framebuffer")


def rpc_call(stream, request):
    stream.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
    line = stream.readline()
    if not line:
        raise RuntimeError("guest MCP socket disconnected")
    response = json.loads(line)
    if response.get("jsonrpc") != "2.0" or response.get("id") != request["id"]:
        raise RuntimeError("unexpected MCP response: " + str(response))
    if "error" in response:
        raise RuntimeError("guest MCP error: " + str(response["error"]))
    return response


def state(stream, request_id):
    request = {"jsonrpc": "2.0", "id": request_id, "method": "tools/call", "params":
               {"name": "pointer_state", "arguments": {}}}
    response = rpc_call(stream, request)
    result = response.get("result", {})
    content = result.get("content", [])
    if result.get("isError") is not False or len(content) != 1 or content[0].get("type") != "text":
        raise RuntimeError("unexpected pointer_state response: " + str(response))
    return request, response, content[0].get("text", "")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--qemu',required=True);parser.add_argument('--guest',required=True);parser.add_argument('--output-dir',required=True);parser.add_argument('--transport',choices=['modern','legacy'],default='modern')
    args=parser.parse_args();guest_hash=sha256(args.guest);dest=Path(args.output_dir).resolve();dest.mkdir(parents=True,exist_ok=True)
    version=subprocess.run([args.qemu,'--version'],check=True,capture_output=True,text=True,timeout=10).stdout.splitlines()[0]
    trace=[]; qmp_trace=[]; frames={}
    with tempfile.TemporaryDirectory(prefix='graph-interaction-') as tmp:
        serial=Path(tmp)/'serial';qmp=Path(tmp)/'qmp'
        command=[args.qemu,'-M','virt,virtualization=off','-accel','tcg','-cpu','cortex-a57','-smp','1','-m','128M',
                 '-nodefaults','-display','none','-nic','none','-device','virtio-gpu-device','-device','virtio-tablet-device',
                 '-chardev',f'socket,id=mcp,path={serial},server=on,wait=off','-serial','chardev:mcp',
                 '-monitor','none','-qmp',f'unix:{qmp},server=on,wait=off','-no-reboot','-kernel',args.guest,
                 '-global','virtio-mmio.force-legacy='+('true' if args.transport=='legacy' else 'false')]
        with tempfile.TemporaryFile(mode='w+b') as log:
            process=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT)
            try:
                with connect(qmp,process) as qs, qs.makefile('rwb',buffering=0) as q:
                    if 'QMP' not in json.loads(q.readline()):raise RuntimeError('invalid QMP greeting')
                    qmp_command(q,{'execute':'qmp_capabilities'})
                    wait_for_guest_frame(q,Path(tmp)/'ready.ppm',process)
                    with connect(serial,process) as ss, ss.makefile('rwb',buffering=0) as st:
                        init={'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'graph-interaction-test','version':'0.1'}}}
                        reply=rpc_call(st,init);trace.append({'request':init,'response':reply})
                        if reply.get('result',{}).get('protocolVersion')!='2025-11-25':raise RuntimeError('bad MCP version')
                        st.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n');trace.append({'notification':'notifications/initialized'})
                        listing={'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}}
                        reply=rpc_call(st,listing);trace.append({'request':listing,'response':reply})
                        names={t['name'] for t in reply.get('result',{}).get('tools',[])}
                        if not {'click_ai','interaction_state','move_ai','pointer_state','document_edit','document_merge'}.issubset(names):raise RuntimeError('missing MCP tools')
                        next_id=2
                        def raw(tool,arguments,rejected=False):
                            nonlocal next_id
                            next_id+=1;req={'jsonrpc':'2.0','id':next_id,'method':'tools/call','params':{'name':tool,'arguments':arguments}}
                            res=rpc_call(st,req);trace.append({'request':req,'response':res});result=res.get('result',{})
                            if result.get('isError') is not rejected:raise RuntimeError('bad tool result '+str(res))
                            if rejected:return None
                            content=result.get('content',[])
                            if len(content)!=1 or content[0].get('type')!='text':raise RuntimeError('bad content')
                            return content[0]['text']
                        def call(tool,arguments,rejected=False):
                            result=raw(tool,arguments,rejected)
                            return None if rejected else json.loads(result)
                        def wait_state(expected):
                            for _ in range(3):
                                result=call('interaction_state',{})
                                if result==expected:return
                                time.sleep(.75)
                            raise RuntimeError('interaction state timeout: '+str(result))
                        def event(events):
                            req={'execute':'input-send-event','arguments':{'events':events}}
                            res=qmp_command(q,req);qmp_trace.append({'request':req,'response':res})
                        def screenshot(name):
                            path=dest/(name+'.ppm');qmp_command(q,{'execute':'screendump','arguments':{'filename':str(path)}})
                            frames[name]={'file':path.name,'sha256':sha256(path)}
                        baseline={'human':{'focus':'none','clicks':0,'pressed':False},'assistant':{'focus':'none','clicks':0}}
                        if call('interaction_state',{})!=baseline:raise RuntimeError('initial interaction state')
                        raw('click_ai',{},True)
                        if call('interaction_state',{})!=baseline:raise RuntimeError('rejected click changed state')
                        # Synthetic QMP absolute axes map to human (114, 444), inside ACTION.
                        event([{'type':'abs','data':{'axis':'x','value':8500}},{'type':'abs','data':{'axis':'y','value':24575}}])
                        for _ in range(3):
                            pos=raw('pointer_state',{})
                            if pos=='human=114,444; ai=560,330':break
                            time.sleep(.75)
                        else:raise RuntimeError('human pointer did not arrive: '+pos)
                        screenshot('human-before')
                        event([{'type':'btn','data':{'button':'left','down':True}}])
                        human_down={'human':{'focus':'action','clicks':1,'pressed':True},'assistant':{'focus':'none','clicks':0}}
                        wait_state(human_down);screenshot('human-after')
                        if frames['human-before']['sha256']==frames['human-after']['sha256']:raise RuntimeError('human click did not change framebuffer')
                        event([{'type':'btn','data':{'button':'left','down':False}}])
                        human_up={'human':{'focus':'action','clicks':1,'pressed':False},'assistant':{'focus':'none','clicks':0}}
                        wait_state(human_up)
                        if raw('move_ai',{'x':660,'y':444})!='moved AI pointer to 660,444':raise RuntimeError('AI pointer move')
                        screenshot('ai-before')
                        if call('click_ai',{})!={'focus':'select','clicks':1}:raise RuntimeError('AI click result')
                        both={'human':{'focus':'action','clicks':1,'pressed':False},'assistant':{'focus':'select','clicks':1}}
                        if call('interaction_state',{})!=both:raise RuntimeError('focus not independent')
                        screenshot('ai-after')
                        if frames['ai-before']['sha256']==frames['ai-after']['sha256']:raise RuntimeError('AI click did not change framebuffer')
                        # A second human click exercises a distinct control without changing AI focus.
                        event([{'type':'abs','data':{'axis':'x','value':22000}},{'type':'abs','data':{'axis':'y','value':24575}}])
                        for _ in range(3):
                            pos=raw('pointer_state',{})
                            if pos=='human=269,444; ai=660,444':break
                            time.sleep(.75)
                        else:raise RuntimeError('second human pointer position '+pos)
                        event([{'type':'btn','data':{'button':'left','down':True}}])
                        human_select={'human':{'focus':'select','clicks':2,'pressed':True},'assistant':{'focus':'select','clicks':1}}
                        wait_state(human_select)
                        event([{'type':'btn','data':{'button':'left','down':False}}])
                        human_select['human']['pressed']=False
                        wait_state(human_select)
                        # Clicking outside a button must not change either focus or count.
                        event([{'type':'abs','data':{'axis':'x','value':8500}},{'type':'abs','data':{'axis':'y','value':21000}}])
                        for _ in range(3):
                            pos=raw('pointer_state',{})
                            if pos=='human=114,390; ai=660,444':break
                            time.sleep(.75)
                        else:raise RuntimeError('outside human pointer position '+pos)
                        event([{'type':'btn','data':{'button':'left','down':True}}])
                        outside={'human':{'focus':'select','clicks':2,'pressed':True},'assistant':{'focus':'select','clicks':1}}
                        wait_state(outside)
                        event([{'type':'btn','data':{'button':'left','down':False}}])
                        outside['human']['pressed']=False
                        wait_state(outside)
                        # With a nonzero focus already established, exact outside
                        # edges must reject without changing either actor's state.
                        for x,y in [(440,430),(591,430),(441,429),(441,465)]:
                            if raw('move_ai',{'x':x,'y':y})!=f'moved AI pointer to {x},{y}':raise RuntimeError('boundary pointer move failed')
                            raw('click_ai',{},True)
                            if call('interaction_state',{})!=outside:raise RuntimeError('rejected boundary click changed state')
                        # Both corners of each half-open rectangle are accepted.
                        clicks=1
                        for x,y,focus in [(441,430,'action'),(590,464,'action'),(602,430,'select'),(751,464,'select')]:
                            if raw('move_ai',{'x':x,'y':y})!=f'moved AI pointer to {x},{y}':raise RuntimeError('boundary pointer move failed')
                            clicks+=1
                            if call('click_ai',{})!={'focus':focus,'clicks':clicks}:raise RuntimeError('inside boundary click rejected')
                            expected={'human':outside['human'],'assistant':{'focus':focus,'clicks':clicks}}
                            if call('interaction_state',{})!=expected:raise RuntimeError('boundary focus/counter mismatch')
                        # Existing in-memory document tools remain functional and separate.
                        call('document_edit',{'actor':'human','space':'human-space','key':'note','value':'synthetic'})
                        merge=call('document_merge',{})
                        if not any(x=={'key':'note','value':'synthetic','origin':'human'} for x in merge['fields']):raise RuntimeError('document bridge regression')
            except Exception:
                log.flush();log.seek(0);diagnostic=log.read().decode('utf-8',errors='replace')
                if diagnostic:print('QEMU diagnostic: '+diagnostic)
                raise
            finally:
                process.terminate()
                try:process.wait(timeout=5)
                except subprocess.TimeoutExpired:process.kill();process.wait()
    if sha256(args.guest)!=guest_hash:raise RuntimeError('guest image changed')
    evidence={'guest_sha256':guest_hash,'qemu_version':version,'transport':args.transport,'qmp_input':'synthetic VirtIO tablet events',
              'qmp_trace':qmp_trace,'mcp_trace':trace,'framebuffers':frames,
              'limitations':'logical in-guest controls; no physical human click, authentication, real windows or process isolation'}
    (dest/'interaction-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
    print('PASS synthetic human and MCP clicks, independent focus and QEMU framebuffer changes')

if __name__=='__main__':main()
