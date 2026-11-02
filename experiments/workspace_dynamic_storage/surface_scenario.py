"""Real EL0 surface ownership, framebuffer and input integration scenario."""
import time

def phase(g, name, expected=None):
    for _ in range(300):
        raw = bytes.fromhex(g.call('file_get', {'name': name})['hex'])
        if (expected is None and len(raw) == 8) or raw == expected:
            return raw
        state = g.call('process_status', {})
        slot = 0 if name[0] == 'a' else 1
        if state.get('all_slot_states', [None]*3)[slot] == 'exited':
            raise RuntimeError('actor exited before phase ' + str((name, raw, state)))
        time.sleep(.2)
    raise TimeoutError('phase ' + str((name, expected, raw)))

def gate(g, name, peer, flag):
    payload = int(peer).to_bytes(8, 'little') + flag.encode()
    g.call('file_put_hex', {'name': name, 'hex': payload.hex()})

def pixel(path, x, y):
    data = path.read_bytes(); header = b'P6\n800 600\n255\n'
    if not data.startswith(header): raise RuntimeError('unexpected screenshot format')
    i = len(header) + (y*800+x)*3
    return tuple(data[i:i+3])

def run_surface_scenario(g, out, image, stage_program):
    for _ in range(80):
        status = g.call('el0_status', {})
        if status.get('task_states') == ['exited', 'exited'] and status.get('frame_pool_assigned') == 0: break
        time.sleep(.1)
    else: raise RuntimeError('startup did not release frames ' + str(status))
    g.wait_boot_identity()
    for p in 'ab':
        gate(g, p+'-sgate', 0, '0')
        g.call('file_put', {'name': p+'-sphase', 'data': '0'})
    program = stage_program(g, image)
    actors = [g.call('process_spawn_staged', {'id': program}) for _ in range(2)]
    if [a['slot'] for a in actors] != [0, 1]: raise RuntimeError('actor slots ' + str(actors))
    ids = [int.from_bytes(phase(g, p+'-sphase'), 'little') for p in 'ab']
    if not min(ids)>0 or ids[0]==ids[1]: raise RuntimeError('surface IDs ' + str(ids))
    status = g.call('surface_status', {})['surfaces']
    if [(v['slot'],int(v['id']),v['visible']) for v in status] != [(0,ids[0],True),(1,ids[1],True)]: raise RuntimeError('surface status '+str(status))
    gate(g, 'a-sgate', ids[1], '1'); gate(g, 'b-sgate', ids[0], '1')
    for p in 'ab': phase(g, p+'-sphase', b'r')
    screenshot = out / 'surface-present.ppm'
    expected = [((70,301),(0xce,0x57,0x4f)), ((462,301),(0xff,0xb3,0x47))]
    for _ in range(30):
        g.screenshot(screenshot)
        actual = [pixel(screenshot,*xy) for xy,_ in expected]
        if actual == [want for _,want in expected]: break
        time.sleep(.1)
    else: raise RuntimeError('framebuffer pixels '+str((expected,actual)))
    print('PASS separate EL0 surfaces and neighbor access denied', flush=True)
    g.input([{'type':'abs','data':{'axis':'x','value':6762}}, {'type':'abs','data':{'axis':'y','value':16384}}])
    for _ in range(20):
        if g.pointer().startswith('human=94,320;'): break
        time.sleep(.1)
    else: raise RuntimeError('human pointer ' + g.pointer())
    g.input([{'type':'btn','data':{'button':'left','down':True}}])
    time.sleep(.2)
    g.input([{'type':'btn','data':{'button':'left','down':False}}])
    g.call_text('move_ai', {'x':486,'y':320})
    before = g.call('interaction_state', {})
    delivered = 0
    for _ in range(100):
        result = g.call('click_ai', {})
        if result == {'surface':True, 'delivered':True}: delivered += 1
        elif result == {'surface':True, 'delivered':False, 'reason':'queue full'}: break
        else: raise RuntimeError('unexpected AI click reply ' + str(result))
    else: raise RuntimeError('AI event queue did not fill')
    if delivered == 0 or g.call('interaction_state', {}) != before:
        raise RuntimeError('queue full leaked into underlying interaction')
    gate(g, 'a-sgate', ids[1], 'i'); gate(g, 'b-sgate', ids[0], 'i')
    for p in 'ab': phase(g, p+'-sphase', b'd')
    print('PASS distinct human/AI input and full FIFO is consumed without click-through', flush=True)
    stop = {'slot':0,'incarnation':actors[0]['incarnation_text'],'boot_id':actors[0]['boot_id']}
    if g.call('process_stop', stop).get('stopped') is not True: raise RuntimeError('stop failed')
    g.wait_independent(lambda x:x.get('all_slot_states',[None]*3)[:2]==['exited','ready'], 'neighbor after stop')
    for _ in range(20):
        status=g.call('surface_status', {})['surfaces']
        if [v['slot'] for v in status]==[1]: break
        time.sleep(.1)
    else: raise RuntimeError('stopped surface was not removed '+str(status))
    gate(g, 'a-sgate', ids[0], '0'); g.call('file_put', {'name':'a-sphase','data':'0'})
    replacement = g.call('process_spawn_staged', {'id':program})
    if replacement['slot'] != 0 or replacement['incarnation'] <= actors[0]['incarnation']: raise RuntimeError('bad readmission '+str(replacement))
    new_id = int.from_bytes(phase(g, 'a-sphase'), 'little')
    if new_id in ids or new_id <= 0: raise RuntimeError('surface ID reused '+str((new_id,ids)))
    gate(g, 'a-sgate', ids[0], 'p'); phase(g, 'a-sphase', b'p')
    gate(g, 'b-sgate', ids[0], 'q')
    done = g.wait_independent(lambda x:x.get('slot_states')==['exited','exited'], 'surface actors finished')
    if done.get('outcomes')!=[1,1] or done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0: raise RuntimeError('surface cleanup '+str(done))
    if g.call('surface_status', {})['surfaces']: raise RuntimeError('surfaces survived teardown')
    print('PASS stop, stale ID, readmission and teardown', flush=True)
    return done
