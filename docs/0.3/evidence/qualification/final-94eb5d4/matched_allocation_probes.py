"""Fresh full 22x4 allocation actions with a zero-action control for each path."""
import concurrent.futures,gzip,hashlib,json,os,re,shutil,subprocess,threading,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;old=json.loads((out/'complete-performance.json').read_text());assert old['status']=='failed'
base=root/old['latency_path'];prior=json.loads((base/'manifest.json').read_text());matrix_base=root/prior['source_campaign'];matrix=json.loads((matrix_base/'manifest.json').read_text());assert prior['status']==matrix['status']=='passed'
sha=lambda b:hashlib.sha256(b).hexdigest();stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime());logs=base/('matched-setup-'+stamp);logs.mkdir();(logs/'runner.py').write_bytes(Path(__file__).read_bytes())
for name in ['complete-performance.json','queued-verification.json','after-history.json']:
 j=json.loads((out/name).read_text());assert j['status']=='failed',name
 path=out/(Path(name).stem+'-invalid-shared-setup-control.json');assert not path.exists();path.write_bytes((out/name).read_bytes())
path=base/'allocations-invalid-shared-setup-control.json';assert not path.exists();path.write_bytes((base/'allocations.json').read_bytes())
work=root/'target/matched-allocations'/str(os.getpid());work.mkdir(parents=True);oldwork=root/'target/per-request'/base.name.removeprefix('all-paths-');cpus=sorted(os.sched_getaffinity(0))[2:6];assert len(cpus)==4
names=[v['workload'] for v in json.loads((base/'summary.json').read_text())];assert len(names)==22
m=dict(pid=os.getpid(),status='running',stage='matched-per-workload-allocation-controls',latency_path=old['latency_path'],checks=old['checks'],failed_attempt='complete-performance-invalid-shared-setup-control.json')
a=dict(pid=os.getpid(),status='building',matched_setup_controls=True,actions_per_probe=1000,setup_actions=0,timeout_seconds_per_probe=3600,source_manifest_sha256=sha((base/'manifest.json').read_bytes()),source_campaign=prior['source_campaign'],source_campaign_sha256=sha((matrix_base/'manifest.json').read_bytes()),runner_sha256=sha(Path(__file__).read_bytes()),valgrind=subprocess.check_output(['valgrind','--version'],text=True).strip(),cpu_by_lane=dict(zip(prior['lanes'],cpus)),build_checks=[],checks=[],results={lane:{} for lane in prior['lanes']},setup_results={lane:{} for lane in prior['lanes']},lanes={},live_probes={},limitations=['Every selected workload has its own zero-action setup process and 1000-action process using the same fresh binary, fixed-width argument and equal-length probe environment values.','Fixed setup may differ across workloads and is never subtracted using another workload.','Instrumentation runs in four parallel lanes on distinct recorded CPUs. No Valgrind runtime is interpreted as latency.','Process exit skips destruction; frees are not a leak test. Allocation totals include all allocations performed inside the labeled action.','Prior shared-control attempts and timeout/interruption records remain unqualified for isolated action counts.'])
lock=threading.Lock();stop=threading.Event()
def save_json(path,value):
 temporary=path.with_suffix('.json.new');temporary.write_text(json.dumps(value,indent=2)+'\n');temporary.replace(path)
def save():save_json(out/'complete-performance.json',m);save_json(base/'allocations.json',a)
def record(label,command,cwd,env):
 start=time.monotonic();r=subprocess.run(command,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=900);data=gzip.compress(r.stdout,mtime=0);path=logs/(label+'.log.gz');path.write_bytes(data)
 a['build_checks'].append(dict(label=label,command=command,exit_code=r.returncode,seconds=time.monotonic()-start,artifact=str(path.relative_to(base)),sha256=sha(data)));save();assert r.returncode==0,label
 return r.stdout
save()
try:
 for index,(lane,item) in enumerate(prior['lanes'].items()):
  original=oldwork/lane/'consumer';consumer=work/f'lane{index}'/'consumer';shutil.copytree(original,consumer,ignore=shutil.ignore_patterns('target','__pycache__'))
  source=oldwork/lane/'source';source_hashes={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()};assert source_hashes==matrix['lanes'][lane]['source_sha256']
  path=consumer/'crates/consumer/examples/refactor_bench.rs';code=path.read_text();assert sha(code.encode())==item['benchmark_sha256']
  before='''    if std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok().as_deref()==Some("1") {
        for _ in 0..1000 { action(); }
        std::process::exit(0);
    }'''
  after='''    let allocation_probe = std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok();
    if let Some(mode @ ("0" | "1")) = allocation_probe.as_deref() {
        let actions = if mode == "0" { 0 } else { 1000 };
        for _ in 0..actions { action(); }
        std::process::exit(0);
    }'''
  assert code.count(before)==1;code=code.replace(before,after);path.write_text(code);(logs/(lane+'-benchmark.rs')).write_text(code)
  env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(work/f'lane{index}'/'target'))
  for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:env.pop(k,None)
  raw=record(lane+'-build',['cargo','+1.100.0','build','--example','refactor_bench','--release','--locked','--offline','--message-format=json'],consumer/'crates/consumer',env)
  records=[json.loads(line) for line in raw.splitlines() if line.startswith(b'{')];artifacts=[r for r in records if r.get('reason')=='compiler-artifact'];assert artifacts and all(not r['fresh'] for r in artifacts)
  built=[Path(r['executable']) for r in artifacts if r['target']['name']=='refactor_bench' and r.get('executable')];assert len(built)==1
  binary=work/f'lane{index}-alloc';shutil.copy2(built[0],binary)
  assert sha((consumer/'crates/consumer/Cargo.lock').read_bytes())==item['lock_sha256']
  a['lanes'][lane]=dict(binary_path=str(binary),binary_sha256=sha(binary.read_bytes()),source_path=str(source),source_sha256=source_hashes,consumer_path=str(consumer),consumer_sha256={str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()},benchmark_sha256=sha(code.encode()),original_benchmark_sha256=item['benchmark_sha256'],lock_sha256=item['lock_sha256']);save();print(lane,'fresh allocation binary built',flush=True)
 a['status']='running';save()
 def lane_work(index,lane,item):
  binary=Path(item['binary_path']);assert sha(binary.read_bytes())==item['binary_sha256']
  # Expensive reconstruction remains full scope and runs last, after cheap controls.
  ordered=[n for n in names if n!='checkpoint_resume_256_events_chunk_32_and_root']+['checkpoint_resume_256_events_chunk_32_and_root']
  for name in ordered:
   for count in [0,1000]:
    if stop.is_set():return
    command=['taskset','-c',str(cpus[index]),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')];environment=dict(STATECHRONICLE_BENCH_MULTIPLIER='1',STATECHRONICLE_ALLOCATION_PROBE='0' if count==0 else '1');start=time.monotonic()
    child=subprocess.Popen(command,cwd=root,env=dict(os.environ,**environment),stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    with lock:a['live_probes'][lane]=dict(pid=child.pid,workload=name,actions=count);save()
    timed_out=False
    try:raw=child.communicate(timeout=3600)[0]
    except subprocess.TimeoutExpired:timed_out=True;child.kill();raw=child.communicate()[0]
    data=gzip.compress(raw,mtime=0);path=logs/(lane+'-'+name+'-'+str(count)+'.log.gz');path.write_bytes(data);check=dict(lane=lane,workload=name,actions=count,command=command,extra_environment=environment,exit_code=child.returncode,timed_out=timed_out,seconds=time.monotonic()-start,artifact=str(path.relative_to(base)),sha256=sha(data))
    with lock:a['checks'].append(check);del a['live_probes'][lane];save()
    if timed_out or child.returncode:stop.set();raise RuntimeError(lane+' '+name+' '+str(count)+' failed')
    assert re.search(rb'ERROR SUMMARY: 0 errors',raw);match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
    total=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]))
    with lock:
     a['setup_results' if count==0 else 'results'][lane][name]=total
     if count==1000:
      setup=a['setup_results'][lane][name];total['control_subtracted_per_action']={k:(total[k]-setup[k])/1000 for k in ['allocations','bytes']};assert all(v>=0 for v in total['control_subtracted_per_action'].values())
     save()
    print(lane,name,count,'passed',flush=True)
  assert sha(binary.read_bytes())==item['binary_sha256']
 with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
  futures=[pool.submit(lane_work,i,lane,item) for i,(lane,item) in enumerate(a['lanes'].items())]
  for future in concurrent.futures.as_completed(futures):future.result()
 assert len(a['checks'])==176 and len({(c['lane'],c['workload'],c['actions']) for c in a['checks']})==176
 for lane,values in a['results'].items():
  assert set(values)==set(names)==set(a['setup_results'][lane])
  for name in ['native_validate_reduce','portable_validate_reduce_hash']:assert all(values[name]['control_subtracted_per_action'][k]==0 for k in ['allocations','bytes'])
 a['status']='passed';save();m.update(status='passed',allocation_manifest=str((base/'allocations.json').relative_to(root)),allocation_manifest_sha256=sha((base/'allocations.json').read_bytes()))
except BaseException as error:a.update(status='failed',error=repr(error));m.update(status='failed',error=repr(error));raise
finally:save()
print('All 88 full actions and 88 workload-matched setup controls passed.',flush=True)
