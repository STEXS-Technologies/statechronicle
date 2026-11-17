"""Finish every candidate history allocation probe on independent CPU cores."""
import concurrent.futures,gzip,hashlib,json,os,re,subprocess,threading,time
from pathlib import Path
root=Path.cwd();driver=Path(__file__).resolve();out=driver.parent;base=out/'history-performance';dest=out/'history-candidate-allocations';sha=lambda b:hashlib.sha256(b).hexdigest()
old=json.loads((dest/'manifest-interrupted-for-parallel-handoff.json').read_text());review=json.loads((dest/'parallel-handoff-review.json').read_text())
assert old['status']=='failed' and old['checks'][-1]['exit_code']==-15
assert review['status']=='intentionally_interrupted_and_preserved' and review['manifest_interrupted_sha256']==sha((dest/'manifest-interrupted-for-parallel-handoff.json').read_bytes())
assert sha((dest/'manifest.json').read_bytes())==review['manifest_interrupted_sha256']
expected=[('control',n) for n in [1000,100,10]]+[(f'{prefix}_{n}',count) for n,count in [(1,1000),(32,100),(256,10)] for prefix in ['verify_signed_history','checkpoint_ack_loss_resume_and_root']]
labels=[f'allocation-candidate-{name}-{count}' for name,count in expected]
positive=[c for c in old['checks'] if c['exit_code']==0];assert [c['label'] for c in positive]==labels[:4]
for c in positive:
 data=(dest/c['artifact']).read_bytes();assert sha(data)==c['sha256'] and b'ERROR SUMMARY: 0 errors' in gzip.decompress(data)
cpus={labels[4]:2,labels[5]:3,labels[6]:4,labels[7]:5,labels[8]:7};topology={}
for cpu in cpus.values():
 p=Path(f'/sys/devices/system/cpu/cpu{cpu}/topology');topology[str(cpu)]={n:(p/n).read_text().strip() for n in ['core_id','physical_package_id','thread_siblings_list']}
assert len({(v['physical_package_id'],v['core_id']) for v in topology.values()})==5
binary=base/'bench-candidate';assert sha(binary.read_bytes())==old['binary_sha256']
m={k:v for k,v in old.items() if k not in ['error','finished_utc','allocations','checks']}
m.update(pid=os.getpid(),status='running',checks=positive,allocations={},parallel_handoff=dict(review_sha256=sha((dest/'parallel-handoff-review.json').read_bytes()),interrupted_manifest_sha256=review['manifest_interrupted_sha256'],parallel_runner_sha256=sha(driver.read_bytes()),cpu_by_label=cpus,topology=topology,started_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),scope='Reuse four completed probes; execute all five remaining probes in full on distinct cores. Allocation-only concurrency is not a latency result.'))
(dest/'runner-parallel.py').write_bytes(driver.read_bytes());lock=threading.RLock()
def save():
 with lock:
  m['checks'].sort(key=lambda c:labels.index(c['label']));temporary=dest/'manifest.json.new';temporary.write_text(json.dumps(m,indent=2)+'\n');temporary.replace(dest/'manifest.json')
save()
def measure(name,count):
 label=f'allocation-candidate-{name}-{count}';cpu=cpus[label];assert sha(binary.read_bytes())==m['binary_sha256']
 command=['taskset','-c',str(cpu),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
 extra=dict(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count));start=time.monotonic();timed_out=False
 try:
  result=subprocess.run(command,cwd=root,env=dict(os.environ,**extra),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=3600);raw=result.stdout;code=result.returncode
 except subprocess.TimeoutExpired as error:raw=error.output or b'';code=None;timed_out=True
 data=gzip.compress(raw,mtime=0);artifact=label+'-parallel.log.gz';(dest/artifact).write_bytes(data)
 c=dict(label=label,command=command,exit_code=code,timed_out=timed_out,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(data),extra_environment=extra)
 with lock:m['checks'].append(c);save()
 assert code==0 and not timed_out,label
 assert re.search(rb'ERROR SUMMARY: 0 errors',raw)
 print(label,'passed',flush=True)
try:
 with concurrent.futures.ThreadPoolExecutor(max_workers=5) as pool:
  futures=[pool.submit(measure,name,count) for name,count in expected[4:]]
  for f in concurrent.futures.as_completed(futures):f.result()
 assert [c['label'] for c in m['checks']]==labels
 for (name,count),c in zip(expected,m['checks']):
  raw=gzip.decompress((dest/c['artifact']).read_bytes());match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
  totals=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));totals['actions']=count;m['allocations'][name+'@'+str(count)]=totals
 for name,totals in m['allocations'].items():
  count=totals['actions'];control=m['allocations']['control@'+str(count)];totals['control_subtracted_per_action']={key:(totals[key]-control[key])/count for key in ['allocations','bytes']}
 m['status']='passed'
except BaseException as error:m.update(status='failed',error=repr(error));raise
finally:m['finished_utc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime());save()
print('All nine candidate allocation probes passed; independent verification remains required.',flush=True)
