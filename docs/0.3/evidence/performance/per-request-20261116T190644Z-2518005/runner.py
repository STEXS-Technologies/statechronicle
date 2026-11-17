"""Individual-request timing for heavy compatibility workflows; immutable four lanes."""
import gzip,hashlib,io,json,os,pathlib,platform,shutil,statistics,subprocess,tarfile,time
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/performance/matrix-20261116T190518Z-2504159';prior=json.loads((base/'manifest.json').read_text());assert prior['status']=='passed'
stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-'+str(os.getpid());out=base.parent/('per-request-'+stamp);out.mkdir();run=root/'target/per-request'/stamp;run.mkdir(parents=True)
def sha(b):return hashlib.sha256(b).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
def unpack(data,path):
 path.mkdir(parents=True)
 with tarfile.open(fileobj=io.BytesIO(data)) as handle:handle.extractall(path,filter='data')
cpu=prior['cpu'];env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(run/'build'))
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:env.pop(key,None)
def conditions():
 frequency=pathlib.Path(f'/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq')
 return dict(utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),cpu=cpu,load_average=os.getloadavg(),kernel=platform.release(),frequency=frequency.read_text().strip() if frequency.exists() else None)
m=dict(source_campaign_sha256=sha((base/'manifest.json').read_bytes()),source_campaign=str(base.relative_to(root)),candidate_commit=prior['candidate_commit'],cpu=cpu,rustc=subprocess.check_output(['rustc','+1.100.0','-vV'],text=True),environment={k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},status='running',checks=[],lanes={},conditions_before=conditions(),cargo_config_sha256={str(p):sha(p.read_bytes()) for p in [root/'.cargo/config.toml',pathlib.Path.home()/'.cargo/config.toml'] if p.exists()},limitations=['Actual individual action timings, including existing labeled setup and assertions; timer floor measured separately, never subtracted.','31 times 100 actions for single/refusal/planning, 31 times 40 for trade; 10 warmups in every process.','Shared host with CPU affinity, rotating/reversing lane order, seven cycles; no observations discarded.','No overlapping compilation or qualification from this session during timing.','Fixed in-memory compatibility adapters do not measure full canonical facade, real database concurrency or power loss.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,cwd,extra=None):
 start=time.monotonic()
 try:r=subprocess.run(command,cwd=cwd,env=dict(env,**(extra or {})),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180);data=r.stdout;code=r.returncode
 except subprocess.TimeoutExpired as error:data=error.stdout or b'';code='timeout'
 compressed=gzip.compress(data,mtime=0);artifact=label+'.log.gz';(out/artifact).write_bytes(compressed);m['checks'].append(dict(label=label,command=command,cwd=str(cwd),extra_environment=extra or {},exit_code=code,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(compressed)));save()
 if code!=0:raise RuntimeError(f'{label}: {code}; see {artifact}')
 return data
save()
try:
 fixtures=run/'fixtures';unpack(git('archive',prior['candidate_commit'],'--','ci/compat-0.2','ci/wire-0.2'),fixtures)
 binaries={};source_checks={}
 for index,(lane,old) in enumerate(prior['lanes'].items()):
  source=run/lane/'source';data=git('archive',old['revision'],'--',*prior['archive_paths']) if 'archive_paths' in prior else git('archive',old['revision'],'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz')
  assert sha(data)==old['archive_sha256'];unpack(data,source)
  for patch in old['manifest_patches']:
   data=(base/('dependency-control-'+patch['path'].replace('/','_'))).read_bytes();assert sha(data)==patch['after_sha256'];(source/patch['path']).write_bytes(data)
  assert all(sha((source/p).read_bytes())==value for p,value in old['source_sha256'].items());source_checks[lane]=(source,old['source_sha256'])
  consumer_root=source.parent/'consumer';shutil.copytree(fixtures/'ci/compat-0.2',consumer_root);consumer=consumer_root/'crates/consumer';cargo=consumer/'Cargo.toml';cargo.write_text(cargo.read_text().replace('../../../../crates/statechronicle',str(source/'crates/statechronicle')))
  (consumer/'Cargo.lock').write_bytes((base/(lane+'.lock')).read_bytes());(consumer_root/'wire').mkdir();shutil.copyfile(fixtures/'ci/wire-0.2/history.json',consumer_root/'wire/history.json')
  code=(base/(lane+'-benchmark.rs')).read_text();assert sha(code.encode())==old['benchmark_sha256'];start=code.index('fn measure(');end=code.index('\nfn payload(',start)
  function='''fn measure(name: &str, iterations: u32, only: &Option<String>, mut action: impl FnMut()) -> Option<Value> {
    if only.as_ref().is_some_and(|requested| requested != name) { return None; }
    let multiplier: u32 = std::env::var("STATECHRONICLE_BENCH_MULTIPLIER").unwrap().parse().unwrap();
    assert!((1..=20).contains(&multiplier));
    let iterations = iterations.checked_mul(multiplier).unwrap();
    for _ in 0..10 { action(); }
    let mut samples = Vec::with_capacity(31 * iterations as usize);
    for _ in 0..31 {
        for _ in 0..iterations {
            let start = Instant::now();
            action();
            samples.push(start.elapsed().as_nanos() as f64);
        }
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    Some(json!({"workload": name, "timing_kind": "individual action", "sample_count": n,
        "iterations_per_sample": 1, "median_ns": samples[n/2],
        "p95_ns": samples[(n * 95).div_ceil(100)-1], "p99_ns": samples[(n * 99).div_ceil(100)-1],
        "samples_ns_per_operation": samples}))
}
'''
  code=code[:start]+function+code[end:];(consumer/'examples/refactor_bench.rs').write_text(code);(out/(lane+'-benchmark.rs')).write_text(code)
  check(lane+'-build',['cargo','+1.100.0','build','--example','refactor_bench','--release','--locked','--offline'],consumer)
  binary=run/f'lane-{index}-bench';shutil.copy2(pathlib.Path(env['CARGO_TARGET_DIR'])/'release/examples/refactor_bench',binary);binaries[lane]=binary
  assert sha((consumer/'Cargo.lock').read_bytes())==old['lock_sha256']
  m['lanes'][lane]=dict(revision=old['revision'],benchmark_sha256=sha(code.encode()),lock_sha256=old['lock_sha256'],binary_sha256=sha(binary.read_bytes()),binary_bytes=binary.stat().st_size)
  save();print(lane,'built',flush=True)
 assert len({lane['benchmark_sha256'] for lane in m['lanes'].values()})==1
 workloads=['control','authenticated_prepare_including_adapter_setup','authenticated_two_intent_batch_to_refusing_sink_including_setup','two_intent_planning_including_adapter_setup','cross_tenant_trade_plan_including_seed_setup']
 results={name:{lane:[] for lane in binaries} for name in workloads};m['conditions_at_measurement_start']=conditions();save();lanes=list(binaries)
 for cycle in range(7):
  order=lanes[cycle%4:]+lanes[:cycle%4]
  if cycle%2==0:order.reverse()
  for lane in order:
   for name in workloads:
    raw=check(f'cycle-{cycle}-{lane}-{name}',['taskset','-c',str(cpu),str(binaries[lane]),name],root,{'STATECHRONICLE_BENCH_MULTIPLIER':'1' if name=='control' else '20'})
    values=json.loads(raw)['workloads'];assert len(values)==1 and values[0]['workload']==name and values[0]['timing_kind']=='individual action'
    assert values[0]['sample_count']==(1240 if name=='cross_tenant_trade_plan_including_seed_setup' else 3100)
    results[name][lane].append(values[0])
  print('cycle',cycle,'complete',flush=True)
 summary=[]
 for name,lanes in results.items():
  comparisons={}
  for control,test in [('original','dependency-control'),('dependency-control','candidate'),('completion-control','candidate'),('original','candidate')]:
   deltas={metric:[(c[metric]/b[metric]-1)*100 for b,c in zip(lanes[control],lanes[test])] for metric in ['median_ns','p95_ns','p99_ns']}
   comparisons[control+' -> '+test]=dict(paired_percent_deltas=deltas,median_paired_percent_deltas={metric:statistics.median(values) for metric,values in deltas.items()})
  summary.append(dict(workload=name,observations=lanes,comparisons=comparisons))
 (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
 for lane,(source,hashes) in source_checks.items():assert all(sha((source/p).read_bytes())==value for p,value in hashes.items()),lane
 assert not git('diff',prior['candidate_commit'],'--','crates','Cargo.toml','Cargo.lock','fuzz')
 m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['conditions_after']=conditions();m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
