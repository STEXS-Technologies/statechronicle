import gzip,hashlib,io,json,os,pathlib,re,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
out=root/'docs/0.3/evidence/conformance'/('concurrent-mixed-'+revision[:7]);out.mkdir()
def sha(b):return hashlib.sha256(b).hexdigest()
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo','ci/conformance']
archive=subprocess.check_output(['git','archive',revision,'--',*paths])
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/concurrent-qualification'))
m={'candidate':revision,'archive_paths':paths,'archive_sha256':sha(archive),'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'rustc':subprocess.check_output(['rustc','-vV'],text=True),'checks':[],'status':'running'}
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,cmd,work,expected=0,extra=None):
 start=time.monotonic();r=subprocess.run(cmd,cwd=work,env=dict(env,**(extra or {})),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
 data=gzip.compress(r.stdout,mtime=0);name=label+'.log.gz';(out/name).write_bytes(data)
 m['checks'].append({'label':label,'command':cmd,'extra_environment':extra or {},'exit_code':r.returncode,'expected_exit_code':expected,'seconds':time.monotonic()-start,'artifact':name,'sha256':sha(data)});save();print(label,r.returncode,flush=True)
 if r.returncode!=expected:raise RuntimeError(label+' unexpected exit')
 return r.stdout.decode()
save()
try:
 with tempfile.TemporaryDirectory(prefix='statechronicle-concurrent-qualification-') as temporary:
  work=pathlib.Path(temporary)
  with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(work,filter='data')
  source={str(p.relative_to(work)):sha(p.read_bytes()) for p in work.rglob('*') if p.is_file()};m['source_sha256']=source;save()
  common=['--manifest-path','ci/conformance/Cargo.toml','--locked','--offline']
  check('fmt',['cargo','fmt','--manifest-path','ci/conformance/Cargo.toml','--','--check'],work)
  text=check('all-tests',['cargo','test',*common,'--all-targets','--all-features','--','--test-threads=1','--nocapture'],work)
  if sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text)))!=26:raise RuntimeError('test count drift')
  for seed in [0,1,7,42,7331,65537,18446744073709551614,18446744073709551615]:
   if 'mixed concurrent seed='+str(seed)+' waves=' not in text:raise RuntimeError('missing seed')
  for marker in ['EagerEvents: mixed.concurrent.staged_effects','LoseSinglesOnBatch: lookup.original','LoseBatchesOnSingle: mixed.membership','AppendProjections: mixed.concurrent.visibility']:
   if 'mixed concurrent negative '+marker not in text:raise RuntimeError('missing intended oracle')
  check('doctests',['cargo','test',*common,'--doc','--all-features'],work)
  check('strict-clippy',['cargo','clippy',*common,'--all-targets','--all-features','--','-D','warnings'],work)
  check('strict-rustdoc',['cargo','doc',*common,'--no-deps','--all-features'],work,extra={'RUSTDOCFLAGS':'-D warnings'})
  if any(sha((work/p).read_bytes())!=h for p,h in source.items()):raise RuntimeError('qualification source drift')
  # Deliberately reintroduce a harness deadlock: do not commit until every writer
  # has staged. The serialized-writer test must detect the scheduling defect.
  p=work/'ci/conformance/src/batch/history.rs';s=p.read_text();old='result = jobs.join_next(), if !jobs.is_empty() =>';new='result = jobs.join_next(), if !jobs.is_empty() && staging.is_empty() =>'
  if s.count(old)!=1:raise RuntimeError('deadlock mutation anchor drift')
  p.write_text(s.replace(old,new));(out/'deadlock.patch').write_text(old+'\nREPLACED WITH\n'+new+'\n')
  p=work/'ci/conformance/tests/reference.rs';s=p.read_text();a=s.index('async fn concurrent_history_allows_a_serialized_writer_without_staging_deadlock()');part=s[a:];old='std::time::Duration::from_secs(30)'
  if part.count(old)!=1:raise RuntimeError('timeout anchor drift')
  p.write_text(s[:a]+part.replace(old,'std::time::Duration::from_secs(2)'))
  text=check('negative-harness-deadlock',['cargo','test',*common,'--all-targets','concurrent_history_allows_a_serialized_writer_without_staging_deadlock','--','--nocapture'],work,101)
  if 'Elapsed(())' not in text or 'concurrent_history_allows_a_serialized_writer_without_staging_deadlock ... FAILED' not in text:raise RuntimeError('negative did not detect timeout at runtime')
  m['status']='passed_with_scoped_limits'
except BaseException as error:m['status']='failed_or_interrupted';m['error']=repr(error);raise
finally:save()
print(out.relative_to(root),flush=True)
