"""Persistent tree ELF admissions; callers supply sequential disposable VMs."""
import time
def fnv64(data):
    value=0xcbf29ce484222325
    for byte in data:value=((value^byte)*0x100000001b3)&0xffffffffffffffff
    return f'{value:016x}'

def reject(g,tool,args,message):
    g.call(tool,args,rejected=True)
    result=g.trace[-1]['response']['result']
    if result.get('content')!=[{'type':'text','text':message}]:raise RuntimeError('wrong refusal: '+str(result))

def put_tree(g,path,data,generation):
    token=g.call('tree_upload_begin',{'path':path,'size':len(data),'checksum':fnv64(data),'generation':generation})['id']
    for offset in range(0,len(data),512):
        reply=g.call('tree_upload_append',{'id':token,'offset':offset,'hex':data[offset:offset+512].hex()})
        if reply.get('received')!=min(offset+512,len(data)):raise RuntimeError('bad upload progress '+str(reply))
    if g.call('tree_upload_seal',{'id':token})!={'sealed':True}:raise RuntimeError('seal failed')
    result=g.call('tree_upload_commit',{'id':token})
    if result.get('readback_verified') is not True:raise RuntimeError('commit failed '+str(result))
    return result['generation']

def read_tree(g,path,data,generation):
    result=bytearray()
    for off in range(0,len(data),512):
        response=g.call('tree_read',{'path':path,'offset':off,'capacity':min(512,len(data)-off),'generation':generation})
        result.extend(bytes.fromhex(response['hex']))
    if result!=data:raise RuntimeError('tree file changed: '+path)

def spawn(g,path,generation):
    result=g.call('tree_spawn',{'path':path,'generation':generation})
    if result.get('started') is not True or result.get('path')!=path or result.get('generation')!=generation or result.get('slot') not in (0,1,2):raise RuntimeError('bad tree_spawn response '+str(result))
    return result

def stop(g,a):
    result=g.call('process_stop',{'slot':a['slot'],'incarnation':a['incarnation_text'],'boot_id':a['boot_id']})
    if result.get('stopped') is not True:raise RuntimeError('stop failed '+str(result))

def setup(g,image):
    g.progress('before tree executable setup');g.wait_boot_identity()
    reject(g,'tree_spawn',{'path':'/apps/prog','generation':'1'},'tree not opened')
    generation=g.call('tree_provision',{'dedicated':True})['generation']
    for path in ('/apps','/neighbor'):
        generation=g.call('tree_mkdir',{'path':path,'generation':generation})['generation']
    generation=put_tree(g,'/apps/prog',image,generation)
    generation=put_tree(g,'/apps/bad',b'not a valid ELF',generation)
    generation=put_tree(g,'/neighbor/keep',b'unmodified neighbor',generation)
    for name in ('a-gate','b-gate','c-gate'):g.call('file_put',{'name':name,'data':'0'})
    return generation

def verify_bad(g,generation):
    before=g.call('process_status',{})
    for path,revision,error in [('/apps/prog',str(int(generation)+1),'stale tree generation'),
                                ('/missing',generation,'tree entry not found'),('/apps',generation,'tree entry is directory'),
                                ('/apps/bad',generation,'invalid ELF'),('/../bad',generation,'invalid tree path')]:
        reject(g,'tree_spawn',{'path':path,'generation':revision},error)
    after=g.call('process_status',{})
    for field in ('all_incarnations','frame_pool_assigned'):
        if before.get(field)!=after.get(field):raise RuntimeError('refused spawn changed '+field)
    if g.call('tree_status',{}).get('generation')!=generation:raise RuntimeError('refused spawn changed tree revision')

def first_boot(g,image):
    generation=setup(g,image);verify_bad(g,generation)
    actors=[spawn(g,'/apps/prog',generation) for _ in range(3)]
    if [a['slot'] for a in actors]!=[0,1,2]:raise RuntimeError('unexpected actor slots '+str(actors))
    ready=g.wait_independent(lambda s:s.get('all_slot_states')==['ready']*3 and min(s.get('all_io_completed',[0]*3))>0,'tree executables active')
    reject(g,'tree_spawn',{'path':'/apps/prog','generation':generation},'no free process slot')
    stop(g,actors[0]);before=g.call('process_status',{})['all_io_completed'][1]
    g.wait_independent(lambda s:s.get('all_slot_states',[None]*3)==['exited','ready','ready'] and s.get('all_io_completed',[0]*3)[1]>before,'neighbors active after stop')
    verify_bad(g,generation)
    a=spawn(g,'/apps/prog',generation)
    if a['slot']!=0 or a['incarnation']<=actors[0]['incarnation']:raise RuntimeError('bad readmission '+str(a))
    g.wait_independent(lambda s:s.get('all_slot_states')==['ready']*3 and s.get('all_io_completed',[0]*3)[0]>0,'readmitted tree executable')
    for name in ('a-gate','b-gate','c-gate'):g.call('file_put',{'name':name,'data':'1'})
    done=g.wait_independent(lambda s:s.get('all_slot_states')==['exited']*3,'tree executables exited')
    if done.get('all_exit_codes')!=[0,0,0] or done.get('frame_pool_assigned')!=0:raise RuntimeError('bad cleanup '+str(done))
    read_tree(g,'/apps/prog',image,generation);read_tree(g,'/neighbor/keep',b'unmodified neighbor',generation)
    return generation,actors[0]['boot_id']

def second_boot(g,image,generation,old_boot):
    g.progress('before tree executable reopen');g.wait_boot_identity()
    reject(g,'tree_spawn',{'path':'/apps/prog','generation':generation},'tree not opened')
    if g.call('tree_open',{'dedicated':True}).get('generation')!=generation:raise RuntimeError('tree generation not restored')
    read_tree(g,'/apps/prog',image,generation);read_tree(g,'/neighbor/keep',b'unmodified neighbor',generation)
    a=spawn(g,'/apps/prog',generation)
    if a['slot']!=0 or a['boot_id']==old_boot:raise RuntimeError('unexpected boot identity '+str(a))
    g.wait_independent(lambda s:s.get('all_slot_states',[None]*3)[0]=='ready' and s.get('all_io_completed',[0]*3)[0]>0,'persisted executable ready')
    g.call('file_put',{'name':'a-gate','data':'1'})
    done=g.wait_independent(lambda s:s.get('all_slot_states',[None]*3)[0]=='exited','persisted executable exited')
    if done.get('all_exit_codes',[None]*3)[0]!=0 or done.get('frame_pool_assigned')!=0:raise RuntimeError('bad reboot execution '+str(done))
    verify_bad(g,generation);read_tree(g,'/neighbor/keep',b'unmodified neighbor',generation)
