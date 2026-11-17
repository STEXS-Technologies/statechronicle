"""Repeat complete allocation instrumentation with unchanged latency binaries."""
import concurrent.futures,gzip,hashlib,json,os,re,subprocess,threading,time
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;old=json.loads((out/'complete-performance-failed-allocation-timeout.json').read_text());base=root/old['latency_path'];prior=json.loads((base/'manifest.json').read_text());assert prior['status']=='passed'
sha=lambda b:hashlib.sha256(b).hexdigest();stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime());logs=base/('allocation-retry-'+stamp);logs.mkdir();lock=threading.Lock();stop=threading.Event()
names=[v['workload'] for v in json.loads((base/'summary.json').read_text())];assert len(names)==22
run=root/'target/per-request'/base.name.removeprefix('all-paths-');cpus=sorted(os.sched_getaffinity(0))[:4];assert len(cpus)==4
m=dict(pid=os.getpid(),status='running',stage='retry-all-path-allocations',latency_path=old['latency_path'],checks=old['checks'],failed_attempt='complete-performance-failed-allocation-timeout.json')
a=dict(status='running',source_manifest_sha256=sha((base/'manifest.json').read_bytes()),actions_per_probe=1000,timeout_seconds_per_probe=1200,parallel_lanes=4,cpu_by_lane=dict(zip(prior['lanes'],cpus)),valgrind=subprocess.check_output(['valgrind','--version'],text=True).strip(),retry_of='allocations-failed-timeout.json',checks=[],results={lane:{} for lane in prior['lanes']},live_probes={},limitations=['All 22 workloads, all four unchanged binaries, 1000 actions per probe. The initial 180-second timeout is retained as a failed attempt.','Four allocation-only processes may run concurrently on distinct recorded CPUs; no latency inference is made from instrumentation.','Fixed setup and process exit/destruction limitations match the original driver; equal-length executable paths and padded arguments control startup allocation.'])
def save_json(path,d):
    temporary=path.with_suffix(path.suffix+'.new');temporary.write_text(json.dumps(d,indent=2)+'\n');temporary.replace(path)
def save():save_json(out/'complete-performance.json',m);save_json(base/'allocations.json',a)
save()
def lane_work(index,lane,item):
    binary=run/f'lane-{index}-bench';assert sha(binary.read_bytes())==item['binary_sha256']
    for name in names:
        if stop.is_set():return
        command=['taskset','-c',str(cpus[index]),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')]
        start=time.monotonic();p=subprocess.Popen(command,cwd=root,env=dict(os.environ,STATECHRONICLE_BENCH_MULTIPLIER='1',STATECHRONICLE_ALLOCATION_PROBE='1'),stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        with lock:a['live_probes'][lane]=dict(pid=p.pid,workload=name);save()
        timed_out=False
        try:raw=p.communicate(timeout=1200)[0]
        except subprocess.TimeoutExpired:timed_out=True;p.kill();raw=p.communicate()[0]
        compressed=gzip.compress(raw,mtime=0);path=logs/(lane+'-'+name+'.log.gz');path.write_bytes(compressed)
        check=dict(lane=lane,workload=name,command=command,exit_code=p.returncode,timed_out=timed_out,seconds=time.monotonic()-start,artifact=str(path.relative_to(base)),sha256=sha(compressed))
        with lock:a['checks'].append(check);del a['live_probes'][lane];save()
        if timed_out or p.returncode:stop.set();raise RuntimeError(lane+' '+name+' failed')
        assert re.search(rb'ERROR SUMMARY: 0 errors',raw)
        match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
        totals=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]))
        with lock:a['results'][lane][name]=totals;save()
        print(lane,name,'passed',flush=True)
    control=a['results'][lane]['control']
    for total in a['results'][lane].values():total['control_subtracted_per_action']={k:(total[k]-control[k])/1000 for k in ['allocations','bytes']}
    for name in ['native_validate_reduce','portable_validate_reduce_hash']:assert all(a['results'][lane][name][k]==control[k] for k in ['allocations','bytes'])
    assert sha(binary.read_bytes())==item['binary_sha256']
    with lock:save()
try:
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        futures=[pool.submit(lane_work,i,lane,item) for i,(lane,item) in enumerate(prior['lanes'].items())]
        for future in concurrent.futures.as_completed(futures):future.result()
    assert len(a['checks'])==88 and all(len(v)==22 for v in a['results'].values())
    a['status']='passed';save();m.update(status='passed',allocation_manifest=str((base/'allocations.json').relative_to(root)),allocation_manifest_sha256=sha((base/'allocations.json').read_bytes()))
except BaseException as error:a.update(status='failed',error=repr(error));m.update(status='failed',error=repr(error));raise
finally:save()
print('All 88 allocation probes passed with the original binaries and full action counts.',flush=True)
