"""Four-lane immutable-source performance observations; run from repository root."""
import gzip,hashlib,io,json,os,pathlib,platform,shutil,statistics,subprocess,tarfile,time,tomllib
root=pathlib.Path.cwd();candidate='a889f172a65bf203a0dde989d372dd111ae4b73e';original='c418e98b91720188158c531cd5c2c4b8ab71cbcf';completion='404dda5817702af2f7ca8700e1a4e2da812e7598'
stamp=time.strftime('%Y%m%dT%H%M%SZ',time.gmtime())+'-'+str(os.getpid());out=root/'docs/0.3/evidence/performance'/('matrix-'+stamp);out.mkdir(parents=True)
run=root/'target/performance-matrix'/stamp;run.mkdir(parents=True)
cpu=min(os.sched_getaffinity(0));env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/performance-matrix/build'))
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:env.pop(key,None)
def sha(b):return hashlib.sha256(b).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
def unpack(data,path):
 path.mkdir(parents=True)
 with tarfile.open(fileobj=io.BytesIO(data)) as handle:handle.extractall(path,filter='data')
def packages(path):return {(p['name'],p['version'],p.get('source'),p.get('checksum')) for p in tomllib.loads(path.read_text())['package'] if p.get('source')}
def conditions():
 base=pathlib.Path(f'/sys/devices/system/cpu/cpu{cpu}/cpufreq')
 return dict(utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),load_average=os.getloadavg(),kernel=platform.release(),cpu=cpu,governor=(base/'scaling_governor').read_text().strip() if (base/'scaling_governor').exists() else None,scaling_cur_freq=(base/'scaling_cur_freq').read_text().strip() if (base/'scaling_cur_freq').exists() else None,cpu_model=next((l.split(':',1)[1].strip() for l in pathlib.Path('/proc/cpuinfo').read_text().splitlines() if l.startswith('model name')),None))
m=dict(candidate_commit=candidate,original_commit=original,completion_control_commit=completion,rustc=subprocess.check_output(['rustc','+1.100.0','-vV'],text=True),cpu=cpu,environment={k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},status='running',checks=[],lanes={},conditions_before=conditions(),limitations=['Shared host; CPU affinity does not isolate interrupts or other users. No discarded samples.','Each p95/p99 is a percentile of 31 batch averages, not individual-request latency.','Original dependencies are retained in original lane; dependency-control changes manifests/lock only, never original Rust.','Compatibility benchmark actions and in-memory adapters do not establish physical durability, production concurrency, or complete canonical-facade performance.','Full default 22-workload matrix and separate 20x iteration regime for the four coordinator/control workloads.','No simultaneous qualification or compilation from this session during latency measurements.'])
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
 # Freeze consumer files and old-wire provenance from the exact candidate tree.
 fixture_data=git('archive',candidate,'--','ci/compat-0.2','ci/wire-0.2')
 fixtures=run/'fixtures';unpack(fixture_data,fixtures);fixture=fixtures/'ci/compat-0.2'
 provenance=json.loads((fixture/'provenance.json').read_text())
 assert provenance['baseline_commit']==original
 for name,item in provenance['files'].items():assert sha(git('show',f"{original}:{item['original_path']}"))==item['sha256'] and sha((fixture/name).read_bytes())==item['sha256']
 wire=fixtures/'ci/wire-0.2/history.json';wire_origin=json.loads((fixtures/'ci/wire-0.2/provenance.json').read_text());assert sha(wire.read_bytes())==wire_origin['history_sha256']
 m.update(fixture_archive_sha256=sha(fixture_data),wire_sha256=sha(wire.read_bytes()),fixture_sha256={str(p.relative_to(fixture)):sha(p.read_bytes()) for p in fixture.rglob('*') if p.is_file()})
 paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz'];binaries={};source_checks={};registry_features={};locks={}
 for index,(lane,revision) in enumerate([('original',original),('dependency-control',original),('completion-control',completion),('candidate',candidate)]):
  source=run/lane/'source';data=git('archive',revision,'--',*paths);unpack(data,source)
  before={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
  patched=[]
  if lane=='dependency-control':
   # Exactly the dependency refresh's manifest surface, plus unused SHA-2
   # dependency placement. No Rust file is changed or copied from candidate.
   for name in ['Cargo.toml','Cargo.lock','crates/statechronicle-core/Cargo.toml','crates/statechronicle-domain/Cargo.toml','crates/statechronicle-portable/Cargo.toml','crates/statechronicle-commit/Cargo.toml']:
    prior=(source/name).read_bytes();replacement=git('show',candidate+':'+name);(source/name).write_bytes(replacement);patched.append(dict(path=name,before_sha256=sha(prior),after_sha256=sha(replacement)))
    (out/('dependency-control-'+name.replace('/','_'))).write_bytes(replacement)
   (out/'dependency-control-manifest.diff').write_bytes(git('diff',original,candidate,'--',*[p['path'] for p in patched if p['path']!='Cargo.lock']))
  after={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
  assert all(before[p]==after[p] for p in before if p.endswith('.rs'))
  source_checks[lane]=(source,after)
  consumer_root=source.parent/'consumer';shutil.copytree(fixture,consumer_root)
  consumer=consumer_root/'crates/consumer';cargo=consumer/'Cargo.toml';cargo.write_text(cargo.read_text().replace('../../../../crates/statechronicle',str(source/'crates/statechronicle')))
  shutil.copyfile(source/'Cargo.lock',consumer/'Cargo.lock');(consumer_root/'wire').mkdir();shutil.copyfile(wire,consumer_root/'wire/history.json')
  bench=consumer/'examples/refactor_bench.rs';code=bench.read_text();needle='    for _ in 0..10 {\n        action();\n    }'
  assert code.count(needle)==1
  code=code.replace(needle,'''    let multiplier: u32 = std::env::var("STATECHRONICLE_BENCH_MULTIPLIER")
        .unwrap_or_else(|_| String::from("1")).parse().unwrap();
    assert!((1..=20).contains(&multiplier));
    let iterations = iterations.checked_mul(multiplier).unwrap();
'''+needle)
  bench.write_text(code);(out/(lane+'-benchmark.rs')).write_text(code)
  metadata=json.loads(check(lane+'-resolve',['cargo','+1.100.0','metadata','--offline','--format-version','1'],consumer))
  assert not packages(consumer/'Cargo.lock')-packages(source/'Cargo.lock')
  lock=(consumer/'Cargo.lock').read_bytes();locks[lane]=lock;(out/(lane+'.lock')).write_bytes(lock)
  registry_features[lane]=sorted((node['id'].split('#')[-1],node['features']) for node in metadata['resolve']['nodes'] if node['id'].startswith('registry+'))
  check(lane+'-build',['cargo','+1.100.0','build','--example','refactor_bench','--release','--locked','--offline'],consumer)
  binary=run/f'lane-{index}-bench';shutil.copy2(pathlib.Path(env['CARGO_TARGET_DIR'])/'release/examples/refactor_bench',binary);binaries[lane]=binary
  m['lanes'][lane]=dict(revision=revision,archive_sha256=sha(data),source_sha256=after,manifest_patches=patched,lock_sha256=sha(lock),registry_features=registry_features[lane],benchmark_sha256=sha(code.encode()),binary_sha256=sha(binary.read_bytes()),binary_bytes=binary.stat().st_size)
  save();print(lane,'built',flush=True)
 assert len({m['lanes'][lane]['benchmark_sha256'] for lane in binaries})==1
 assert locks['dependency-control']==locks['candidate']==locks['completion-control'],'matched dependency locks differ'
 assert registry_features['dependency-control']==registry_features['candidate']==registry_features['completion-control'],'matched registry feature sets differ'
 m['conditions_at_measurement_start']=conditions();save()
 extended=['authenticated_prepare_including_adapter_setup','authenticated_two_intent_batch_to_refusing_sink_including_setup','two_intent_planning_including_adapter_setup','cross_tenant_trade_plan_including_seed_setup']
 observations={'default':{},'extended':{name:{lane:[] for lane in binaries} for name in extended}}
 lanes=list(binaries)
 for cycle in range(7):
  order=lanes[cycle%4:]+lanes[:cycle%4]
  if cycle%2:order.reverse()
  for lane in order:
   raw=check(f'default-{cycle}-{lane}',['taskset','-c',str(cpu),str(binaries[lane])],root,{'STATECHRONICLE_BENCH_MULTIPLIER':'1'})
   values=json.loads(raw)['workloads'];assert len(values)==22
   for value in values:observations['default'].setdefault(value['workload'],{name:[] for name in binaries})[lane].append(value)
  print('default cycle',cycle,'complete',flush=True)
 for cycle in range(7):
  order=lanes[cycle%4:]+lanes[:cycle%4]
  if cycle%2==0:order.reverse()
  for lane in order:
   for name in extended:
    raw=check(f'extended-{cycle}-{lane}-{name}',['taskset','-c',str(cpu),str(binaries[lane]),name],root,{'STATECHRONICLE_BENCH_MULTIPLIER':'20'})
    values=json.loads(raw)['workloads'];assert len(values)==1 and values[0]['workload']==name;observations['extended'][name][lane].append(values[0])
  print('extended cycle',cycle,'complete',flush=True)
 summary=[]
 for regime,workloads in observations.items():
  for name,lanes in workloads.items():
   comparisons={}
   for control,test in [('original','dependency-control'),('dependency-control','candidate'),('completion-control','candidate'),('original','candidate')]:
    deltas={metric:[(c[metric]/b[metric]-1)*100 if b[metric] else None for b,c in zip(lanes[control],lanes[test])] for metric in ['median_ns','p95_ns']}
    comparisons[control+' -> '+test]=dict(paired_percent_deltas=deltas,median_paired_percent_deltas={metric:statistics.median(v for v in values if v is not None) for metric,values in deltas.items() if any(v is not None for v in values)})
   summary.append(dict(regime=regime,workload=name,observations=lanes,comparisons=comparisons))
 (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
 for lane,(source,hashes) in source_checks.items():assert all(sha((source/p).read_bytes())==value for p,value in hashes.items()),lane
 assert not git('diff',candidate,'--','crates','Cargo.toml','Cargo.lock','fuzz')
 m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['conditions_after']=conditions();m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
