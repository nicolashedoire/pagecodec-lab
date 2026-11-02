"""Real EL0 keyboard isolation; callers provide a disposable QEMU guest."""
import time
from surface_scenario import phase, gate

def tap_key(g):
    # Only a synthetic, known QEMU key is injected; no host keystrokes are logged.
    for down in (True,False):
        g.input([{'type':'key','data':{'down':down,'key':{'type':'qcode','data':'a'}}}])
        time.sleep(.04)

def human_click(g,x,y):
    # QEMU tablet absolute coordinates map onto the left half of the display.
    raw_x=((x-16)*32767+377)//378;raw_y=((y-70)*32767+499)//500
    g.input([{'type':'abs','data':{'axis':'x','value':raw_x}}, {'type':'abs','data':{'axis':'y','value':raw_y}}])
    for _ in range(20):
        if g.pointer().startswith(f'human={x},{y};'): break
        time.sleep(.1)
    else:raise RuntimeError('pointer failed to reach target '+g.pointer())
    g.input([{'type':'btn','data':{'button':'left','down':True}}]);time.sleep(.1)
    g.input([{'type':'btn','data':{'button':'left','down':False}}]);time.sleep(.1)

def wait_focus(g, focused):
    for _ in range(60):
        status=g.call('surface_status',{})
        slots=[v['slot'] for v in status['surfaces'] if v.get('keyboard_focused')]
        if slots==focused:return
        time.sleep(.1)
    raise RuntimeError('keyboard focus mismatch '+str(status))

def wait_queued(g, count):
    for _ in range(60):
        status=g.call('surface_status',{})
        a=next(v for v in status['surfaces'] if v['slot']==0)
        if a.get('keyboard_queued')==count:return
        time.sleep(.1)
    raise RuntimeError('keyboard queue mismatch '+str(status))

def run_keyboard_scenario(g,image,stage_program,keyboard=True):
    for _ in range(80):
        boot=g.call('el0_status',{})
        if boot.get('task_states')==['exited','exited'] and boot.get('frame_pool_assigned')==0:break
        time.sleep(.1)
    else:raise RuntimeError('startup did not release frames '+str(boot))
    g.wait_boot_identity()
    for p in 'ab':
        gate(g,p+'-kgate',0,'0');g.call('file_put',{'name':p+'-kphase','data':'0'})
    program=stage_program(g,image)
    actors=[g.call('process_spawn_staged',{'id':program}) for _ in range(2)]
    if [a['slot'] for a in actors]!=[0,1]:raise RuntimeError('bad slots '+str(actors))
    ids=[int.from_bytes(phase(g,p+'-kphase'),'little') for p in 'ab']
    gate(g,'a-kgate',ids[1],'1');gate(g,'b-kgate',ids[0],'1')
    for p in 'ab':phase(g,p+'-kphase',b'r')
    status=g.call('surface_status',{})
    if status.get('keyboard_available_at_boot') is not keyboard:raise RuntimeError('keyboard discovery mismatch '+str(status))
    wait_focus(g,[])
    if keyboard:tap_key(g);time.sleep(.25)
    gate(g,'a-kgate',ids[1],'n');gate(g,'b-kgate',ids[0],'n')
    for p in 'ab':phase(g,p+'-kphase',b'n')
    if not keyboard:
        for a in actors:g.call('process_stop',{'slot':a['slot'],'incarnation':a['incarnation_text'],'boot_id':a['boot_id']})
        done=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'keyboard absent clients stopped')
        if done.get('frame_pool_assigned')!=0:raise RuntimeError('keyboard absent leaked frames '+str(done))
        return done
    human_click(g,94,320);wait_focus(g,[0]);wait_queued(g,0)
    g.call_text('move_ai',{'x':486,'y':320})
    if g.call('click_ai',{})!={'surface':True,'delivered':True}:raise RuntimeError('AI click did not reach its surface')
    wait_focus(g,[0]);wait_queued(g,0);tap_key(g);wait_queued(g,2)
    gate(g,'a-kgate',ids[1],'k');gate(g,'b-kgate',ids[0],'k')
    for p in 'ab':phase(g,p+'-kphase',b'k')
    g.input([{'type':'key','data':{'down':True,'key':{'type':'qcode','data':'a'}}}]);wait_queued(g,1)
    human_click(g,80,150);wait_focus(g,[])
    g.input([{'type':'key','data':{'down':False,'key':{'type':'qcode','data':'a'}}}]);wait_queued(g,2)
    gate(g,'a-kgate',ids[1],'t');gate(g,'b-kgate',ids[0],'t')
    for p in 'ab':phase(g,p+'-kphase',b't')
    tap_key(g);time.sleep(.25)
    gate(g,'a-kgate',ids[1],'u');gate(g,'b-kgate',ids[0],'u')
    for p in 'ab':phase(g,p+'-kphase',b'u')
    human_click(g,94,320);wait_focus(g,[0])
    for _ in range(4):tap_key(g)
    wait_queued(g,8)
    # Ninth event overflows the queue, yielding one reset; its unmatched
    # release must be ignored. A subsequent pair reconstructs usable state.
    g.input([{'type':'key','data':{'down':True,'key':{'type':'qcode','data':'a'}}}]);wait_queued(g,1)
    g.input([{'type':'key','data':{'down':False,'key':{'type':'qcode','data':'a'}}}]);time.sleep(.2)
    status=g.call('surface_status',{})
    a=next(s for s in status['surfaces'] if s['slot']==0)
    if a.get('keyboard_queued')!=1 or a.get('keyboard_dropped',0)<1:raise RuntimeError('keyboard reset/ignored release mismatch '+str(status))
    tap_key(g);wait_queued(g,3)
    gate(g,'a-kgate',ids[1],'f');gate(g,'b-kgate',ids[0],'f')
    for p in 'ab':phase(g,p+'-kphase',b'f')
    tap_key(g);wait_queued(g,2)
    stop={'slot':0,'incarnation':actors[0]['incarnation_text'],'boot_id':actors[0]['boot_id']}
    if not g.call('process_stop',stop).get('stopped'):raise RuntimeError('cannot stop owner')
    g.wait_independent(lambda s:s.get('all_slot_states',[None]*3)[:2]==['exited','ready'],'keyboard owner stopped')
    wait_focus(g,[])
    gate(g,'a-kgate',ids[0],'0');g.call('file_put',{'name':'a-kphase','data':'0'})
    new=g.call('process_spawn_staged',{'id':program})
    if new['slot']!=0 or new['incarnation']<=actors[0]['incarnation']:raise RuntimeError('bad readmission '+str(new))
    newid=int.from_bytes(phase(g,'a-kphase'),'little')
    if newid in ids:raise RuntimeError('ID reused')
    gate(g,'a-kgate',ids[0],'p');phase(g,'a-kphase',b'p')
    gate(g,'b-kgate',ids[0],'q')
    done=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'keyboard clients exited')
    if done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('bad keyboard teardown '+str(done))
    return done
