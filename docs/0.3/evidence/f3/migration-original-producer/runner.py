import pathlib,subprocess,tarfile,io,os,json,gzip,hashlib,tempfile
root=pathlib.Path.cwd();revision='c418e98b91720188158c531cd5c2c4b8ab71cbcf'
out=root/'docs/0.3/evidence/f3/migration-original-producer';out.mkdir(parents=True,exist_ok=False)
fixture=root/'crates/statechronicle/tests/fixtures/original_configuration_history.json'
if fixture.exists():raise RuntimeError('refuse corpus overwrite')
def sha(b):return hashlib.sha256(b).hexdigest()
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo']
archive=subprocess.check_output(['git','archive',revision,'--',*paths]);producer=(root/'ci/characterization/migration_history.rs').read_bytes()
m=dict(producer_commit=revision,archive_sha256=sha(archive),producer_sha256=sha(producer),checks=[],status='running',limitations=['Fixed fixture keys and clock, not production authorization.','Original-release producer, unchanged original lock and library source, current Rust compiler.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes());(out/'migration_history.rs').write_bytes(producer)
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 with tempfile.TemporaryDirectory(prefix='statechronicle-migration-original-') as temp:
  source=pathlib.Path(temp)
  with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
  hashes={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()};m['source_sha256']=hashes
  (source/'crates/statechronicle/examples/migration_history.rs').write_bytes(producer)
  env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/migration-original-producer'))
  command=['cargo','run','-p','statechronicle','--example','migration_history','--locked','--offline','--',str(fixture)]
  result=subprocess.run(command,cwd=source,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
  data=gzip.compress(result.stdout,mtime=0);(out/'producer.log.gz').write_bytes(data)
  m['checks'].append(dict(command=command,exit_code=result.returncode,artifact='producer.log.gz',sha256=sha(data)));save()
  if result.returncode:raise RuntimeError('original producer failed')
  if not all(sha((source/p).read_bytes())==h for p,h in hashes.items()):raise RuntimeError('original source or lock drift')
  m['fixture_sha256']=sha(fixture.read_bytes());m['rustc']=subprocess.check_output(['rustc','-vV'],text=True);m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:save()
print('original producer passed',flush=True)
