import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();base=root/'docs/0.3/evidence/conformance'/('concurrent-mixed-'+revision[:7]);out=base/'deadlock-control';out.mkdir()
def sha(b):return hashlib.sha256(b).hexdigest()
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo','ci/conformance'];archive=subprocess.check_output(['git','archive',revision,'--',*paths]);env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/concurrent-qualification'))
m={'candidate':revision,'archive_sha256':sha(archive),'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'status':'running'};(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 with tempfile.TemporaryDirectory(prefix='statechronicle-concurrent-deadlock-') as temporary:
  work=pathlib.Path(temporary)
  with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(work,filter='data')
  m['source_sha256']={str(p.relative_to(work)):sha(p.read_bytes()) for p in work.rglob('*') if p.is_file()};save()
  p=work/'ci/conformance/src/batch/history.rs';s=p.read_text();a=s.index('pub async fn check_concurrent');old=s[a:];new=old.replace('let mut jobs = tokio::task::JoinSet::new();','let mut jobs = tokio::task::JoinSet::new();\n        let commit_barrier = std::sync::Arc::new(tokio::sync::Barrier::new(wave.len()));',1).replace('jobs.spawn(async move {','let commit_barrier = commit_barrier.clone();\n                    jobs.spawn(async move {\n                        commit_barrier.wait().await;',1)
  if old==new or new.count('commit_barrier.wait().await;')!=1:raise RuntimeError('mutation anchor drift')
  p.write_text(s[:a]+new)
  (out/'deadlock.patch').write_text('\n'.join(__import__('difflib').unified_diff(s.splitlines(),(s[:a]+new).splitlines(),fromfile='original',tofile='mutated'))+'\n')
  p=work/'ci/conformance/tests/reference.rs';s=p.read_text();a=s.index('async fn concurrent_history_allows_a_serialized_writer_without_staging_deadlock()');part=s[a:]
  if part.count('std::time::Duration::from_secs(30)')!=1:raise RuntimeError('timeout anchor drift')
  p.write_text(s[:a]+part.replace('std::time::Duration::from_secs(30)','std::time::Duration::from_secs(2)'))
  command=['cargo','test','--manifest-path','ci/conformance/Cargo.toml','--locked','--offline','--all-targets','concurrent_history_allows_a_serialized_writer_without_staging_deadlock','--','--nocapture'];start=time.monotonic();r=subprocess.run(command,cwd=work,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600);data=gzip.compress(r.stdout,mtime=0);(out/'test.log.gz').write_bytes(data);m.update(command=command,exit_code=r.returncode,expected_exit_code=101,seconds=time.monotonic()-start,artifact='test.log.gz',sha256=sha(data));save()
  if r.returncode!=101 or b'Elapsed(())' not in r.stdout or b'concurrent_history_allows_a_serialized_writer_without_staging_deadlock ... FAILED' not in r.stdout:raise RuntimeError('deadlock not detected at runtime')
  m['status']='passed_expected_rejection'
except BaseException as error:m['status']='failed_or_interrupted';m['error']=repr(error);raise
finally:save()
print(out.relative_to(root),m['exit_code'],flush=True)
