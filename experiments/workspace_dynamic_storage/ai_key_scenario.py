"""Draft integration scenario. Caller supplies a disposable QEMU guest and staged ELF.

Never runs a VM on import. Import alongside the existing surface_scenario and
surface_key_scenario modules, and call run_ai_key_scenario(g, image, stage_program).
"""
import time
from surface_scenario import phase, gate
from surface_key_scenario import human_click, tap_key, wait_focus, wait_queued


def expect_error(g, args, expected):
    g.call('key_ai',args,rejected=True)
    result = g.trace[-1]['response'].get('result', {})
    if result.get('isError') is not True or result.get('content') != [{'type':'text','text':expected}]:
        raise RuntimeError('unexpected error: '+str(result))


def run_ai_key_scenario(g, image, stage_program):
    for _ in range(80):
        boot = g.call('el0_status', {})
        if boot.get('task_states') == ['exited','exited'] and boot.get('frame_pool_assigned') == 0:
            break
        time.sleep(.1)
    else:
        raise RuntimeError('startup did not release frames '+str(boot))
    g.wait_boot_identity()
    for p in 'ab':
        gate(g,p+'-kgate',0,'0')
        g.call('file_put',{'name':p+'-kphase','data':'0'})
    program=stage_program(g,image)
    actors=[g.call('process_spawn_staged',{'id':program}) for _ in range(2)]
    if [a['slot'] for a in actors] != [0,1]: raise RuntimeError('bad slots '+str(actors))
    ids=[int.from_bytes(phase(g,p+'-kphase'),'little') for p in 'ab']
    gate(g,'a-kgate',ids[1],'1'); gate(g,'b-kgate',ids[0],'1')
    for p in 'ab': phase(g,p+'-kphase',b'r')
    if not g.call('surface_status',{}).get('keyboard_available_at_boot'):
        raise RuntimeError('test requires a QEMU keyboard')
    gate(g,'a-kgate',ids[1],'n'); gate(g,'b-kgate',ids[0],'n')
    for p in 'ab': phase(g,p+'-kphase',b'n')
    human_click(g,94,320); wait_focus(g,[0])
    g.call_text('move_ai',{'x':486,'y':320})
    if g.call('click_ai',{}) != {'surface':True,'delivered':True}: raise RuntimeError('AI click failed')
    def surface(slot): return next(s for s in g.call('surface_status',{})['surfaces'] if s['slot']==slot)
    if not surface(0)['keyboard_focused'] or not surface(1)['ai_keyboard_focused']:
        raise RuntimeError('independent focus missing')
    args={'id':str(ids[1]),'incarnation':actors[1]['incarnation_text'],
          'boot_id':actors[1]['boot_id'],'code':30,'value':1}
    def send(value): return g.call('key_ai',{**args,'value':value})
    def delivered(value):
        reply=send(value)
        if reply != {'delivered':True}: raise RuntimeError('key delivery '+str(reply))
    before=surface(1)['keyboard_queued']
    stale_boot=('0' if args['boot_id'][0]!='0' else '1')+args['boot_id'][1:]
    expect_error(g,{**args,'boot_id':stale_boot},'stale boot identity')
    expect_error(g,{**args,'incarnation':str(int(args['incarnation'])+1)},'stale process incarnation')
    expect_error(g,{**args,'id':str(ids[0])},'surface access denied')
    expect_error(g,{**args,'id':str(max(ids)+100)},'stale surface')
    expect_error(g,{**args,'value':3},'invalid key_ai arguments')
    if surface(1)['keyboard_queued']!=before: raise RuntimeError('rejected key changed queue')
    delivered(1); delivered(0); tap_key(g); wait_queued(g,2)
    gate(g,'a-kgate',ids[1],'k'); gate(g,'b-kgate',ids[0],'k')
    for p in 'ab': phase(g,p+'-kphase',b'k')
    # Pointer events do not drain while fixture is waiting: fill to capacity.
    while surface(1)['queued']<8:
        if g.call('click_ai',{}) != {'surface':True,'delivered':True}: raise RuntimeError('pointer queue fill failed')
    if g.call('click_ai',{}) != {'surface':True,'delivered':False,'reason':'queue full'}: raise RuntimeError('pointer full missing')
    if not surface(1)['ai_keyboard_focused']: raise RuntimeError('full click cleared existing focus')
    delivered(1)
    g.call_text('move_ai',{'x':500,'y':450}); g.call('click_ai',{})
    if surface(1)['ai_keyboard_focused'] or not surface(0)['keyboard_focused']: raise RuntimeError('focus loss invalid')
    delivered(0) # released to original surface despite loss of focus
    g.call_text('move_ai',{'x':486,'y':320})
    if g.call('click_ai',{}) != {'surface':True,'delivered':False,'reason':'queue full'}: raise RuntimeError('full click unexpectedly accepted')
    if surface(1)['ai_keyboard_focused']: raise RuntimeError('full click restored focus')
    if send(1) != {'delivered':False,'reason':'no focus'}: raise RuntimeError('unfocused press accepted')
    gate(g,'a-kgate',ids[1],'t'); gate(g,'b-kgate',ids[0],'t')
    for p in 'ab': phase(g,p+'-kphase',b't')
    gate(g,'a-kgate',ids[1],'u'); gate(g,'b-kgate',ids[0],'u')
    for p in 'ab': phase(g,p+'-kphase',b'u')
    if surface(1)['queued'] != 0: raise RuntimeError('fixture failed to drain pointer events')
    if g.call('click_ai',{}) != {'surface':True,'delivered':True}: raise RuntimeError('focus reacquisition failed')
    for _ in range(4): delivered(1); delivered(0)
    if send(1) != {'delivered':False,'reason':'queue reset'}: raise RuntimeError('no KEY_RESET')
    if send(0) != {'delivered':False,'reason':'unpaired'}: raise RuntimeError('reset did not clear state')
    delivered(1); delivered(0)
    gate(g,'a-kgate',ids[1],'f'); gate(g,'b-kgate',ids[0],'f')
    for p in 'ab': phase(g,p+'-kphase',b'f')
    if not g.call('process_stop',{'slot':1,'incarnation':actors[1]['incarnation_text'],'boot_id':actors[1]['boot_id']}).get('stopped'):
        raise RuntimeError('failed to stop AI actor')
    g.wait_independent(lambda s:s.get('all_slot_states',[None]*3)[:2]==['ready','exited'],'AI actor stopped')
    expect_error(g,args,'stale process incarnation')
    gate(g,'b-kgate',ids[1],'0');g.call('file_put',{'name':'b-kphase','data':'0'})
    new=g.call('process_spawn_staged',{'id':program})
    if new['slot']!=1 or new['incarnation']<=actors[1]['incarnation']: raise RuntimeError('bad readmission')
    new_id=int.from_bytes(phase(g,'b-kphase'),'little')
    expect_error(g,args,'stale process incarnation')
    expect_error(g,{**args,'incarnation':new['incarnation_text']},'stale surface')
    if new_id in ids or surface(1)['keyboard_queued']!=0 or surface(1)['ai_keyboard_focused']:
        raise RuntimeError('old AI state survived readmission')
    if not surface(0)['keyboard_focused']: raise RuntimeError('human focus lost on AI stop')
    gate(g,'b-kgate',ids[1],'p'); phase(g,'b-kphase',b'p')
    gate(g,'a-kgate',ids[1],'q')
    done=g.wait_independent(lambda s:s.get('slot_states')==['exited','exited'],'keyboard clients exited')
    if done.get('exit_codes')!=[0,0] or done.get('frame_pool_assigned')!=0: raise RuntimeError('bad teardown '+str(done))
    return done
