import gzip,hashlib,io,json,os,pathlib,platform,shutil,statistics,subprocess,sys,tarfile,tomllib
root=pathlib.Path.cwd();candidate='52e8e6fa81167f94998b31ff0980c5549a8d8019';control='79baccfdfae964f8007dc086aa8a2f0d25f022b8';out=root/'docs/0.3/evidence/maintenance/outbox-preflight/paired-52e8e6f';run=root/'target/outbox-performance/52e8e6f';cpu=8
if cpu not in os.sched_getaffinity(0):raise RuntimeError('CPU unavailable')
def sha(b):return hashlib.sha256(b).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
def packages(p):return {(x['name'],x['version'],x.get('source'),x.get('checksum')) for x in tomllib.loads(p.read_text())['package'] if x.get('source')}
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/outbox-performance/build'))
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:env.pop(key,None)
def conditions():return dict(load_average=os.getloadavg(),kernel=platform.release(),cpu=cpu,affinity=sorted(os.sched_getaffinity(0)))
if sys.argv[1]=='prepare':
 out.mkdir(parents=True,exist_ok=False);run.mkdir(parents=True,exist_ok=False)
 m=dict(candidate_commit=candidate,control_commit=control,cpu=cpu,rustc=subprocess.check_output(['rustc','-vV'],text=True),checks=[],lanes={},status='preparing',conditions_before=conditions(),limitations=['Same compiler, byte-identical third-party lock/features, exact equal preflight/refusal workloads.','A sentinel begin refusal measures signed durable preflight, not physical commit latency or protected canonical-facade success.','Shared host, pinned CPU, seven genuinely alternating lane orders; no samples discarded.','Each percentile describes 31 batch averages, not individual request latency.','No concurrent qualification/compilation during the measure phase.'])
 (out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
else:m=json.loads((out/'manifest.json').read_text())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,cwd):
 result=subprocess.run(command,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
 name=label+'.log.gz';data=gzip.compress(result.stdout,mtime=0);(out/name).write_bytes(data)
 m['checks'].append(dict(label=label,command=command,exit_code=result.returncode,artifact=name,sha256=sha(data)));save()
 if result.returncode:raise RuntimeError(label+' failed')
 return result.stdout
try:
 if sys.argv[1]=='prepare':
  fixture=git('show',candidate+':ci/characterization/outbox.rs');support=git('show','c418e98b91720188158c531cd5c2c4b8ab71cbcf:crates/statechronicle/tests/common/harness.rs')
  (out/'outbox.rs').write_bytes(fixture);(out/'common.rs').write_bytes(support)
  locks={};features={}
  for lane,revision in [('control',control),('candidate',candidate)]:
   source=run/lane/'source';source.mkdir(parents=True)
   archive=git('archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz')
   with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
   hashes={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
   consumer=source.parent/'consumer';(consumer/'src').mkdir(parents=True)
   (consumer/'src/main.rs').write_bytes(fixture);(consumer/'src/common.rs').write_bytes(support)
   (consumer/'Cargo.toml').write_text('[package]\nname="outbox-performance"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nstatechronicle={path="../source/crates/statechronicle"}\nserde_json="1"\nchrono="0.4"\nasync-trait="0.1"\ned25519-dalek="3"\ntokio={version="1",features=["rt"]}\n')
   (consumer/'Cargo.lock').write_bytes((source/'Cargo.lock').read_bytes())
   metadata=json.loads(check(lane+'-resolve',['cargo','metadata','--offline','--format-version','1'],consumer))
   if packages(consumer/'Cargo.lock')-packages(source/'Cargo.lock'):raise RuntimeError('dependency drift')
   locks[lane]=(consumer/'Cargo.lock').read_bytes();(out/(lane+'.lock')).write_bytes(locks[lane])
   features[lane]=sorted((n['id'].split('#')[-1],n['features']) for n in metadata['resolve']['nodes'] if n['id'].startswith('registry+'))
   check(lane+'-build',['cargo','build','--release','--locked','--offline'],consumer)
   check(lane+'-clippy',['cargo','clippy','--locked','--offline','--','-D','warnings'],consumer)
   binary=run/(lane+'-bench');shutil.copy2(pathlib.Path(env['CARGO_TARGET_DIR'])/'release/outbox-performance',binary)
   m['lanes'][lane]=dict(revision=revision,archive_sha256=sha(archive),source_sha256=hashes,consumer=str(consumer),binary=str(binary),binary_sha256=sha(binary.read_bytes()),binary_bytes=binary.stat().st_size,lock_sha256=sha(locks[lane]),registry_features=features[lane],fixture_sha256=sha(fixture),support_sha256=sha(support))
   if any(sha((source/p).read_bytes())!=h for p,h in hashes.items()):raise RuntimeError('source drift')
   save();print(lane,'prepared',flush=True)
  if locks['control']!=locks['candidate'] or features['control']!=features['candidate']:raise RuntimeError('unmatched build')
  m['status']='prepared'
 elif sys.argv[1]=='measure':
  if m['status']!='prepared':raise RuntimeError('requires prepared binaries')
  m['conditions_at_measurement_start']=conditions();m['observations']={}
  for cycle in range(7):
   order=['control','candidate'] if cycle%2==0 else ['candidate','control']
   for lane in order:
    info=m['lanes'][lane];binary=pathlib.Path(info['binary'])
    if sha(binary.read_bytes())!=info['binary_sha256']:raise RuntimeError('binary drift')
    values=json.loads(check(f'cycle-{cycle}-{lane}',['taskset','-c',str(cpu),str(binary),'--bench'],root))
    if len(values)!=6 or any('sentinel:begin' not in v['expected_outcome'] for v in values):raise RuntimeError('incomplete workload')
    for value in values:m['observations'].setdefault(value['workload'],{'control':[],'candidate':[]})[lane].append(value)
   print('cycle',cycle,'done',flush=True)
  m['summary']={}
  for name,lanes in m['observations'].items():
   summary={lane:{key:statistics.median(v[key] for v in values) for key in ['median_ns','p95_ns']} for lane,values in lanes.items()}
   summary['paired_change_percent']={key:statistics.median((new[key]/old[key]-1)*100 for old,new in zip(lanes['control'],lanes['candidate'])) for key in ['median_ns','p95_ns']}
   m['summary'][name]=summary
  m['conditions_at_measurement_end']=conditions();m['status']='passed'
 else:raise RuntimeError('unknown phase')
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:save()
