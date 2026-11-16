import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd(); revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
out=root/'docs/0.3/evidence/f1/protected-settlement'/('qualification-'+revision[:7]);out.mkdir(exist_ok=False)
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo']
archive=subprocess.check_output(['git','archive',revision,'--',*paths])
def sha(data):return hashlib.sha256(data).hexdigest()
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/builtin-qualification'))
m=dict(candidate_commit=revision,archive_sha256=sha(archive),archive_paths=paths,environment={k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},rustc=subprocess.check_output(['rustc','-vV'],text=True),checks=[],status='running',limitations=['Finite root tests and static checks; fuzz targets compiled by Clippy, never executed as unbounded workspace tests.','Not a complete release gate, final coverage or complete external backend qualification.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,cwd,extra=None):
 start=time.monotonic();result=subprocess.run(command,cwd=cwd,env=dict(env,**(extra or {})),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
 data=gzip.compress(result.stdout,mtime=0);name=label+'.log.gz';(out/name).write_bytes(data)
 m['checks'].append(dict(label=label,command=command,cwd=str(cwd),extra_environment=extra or {},exit_code=result.returncode,seconds=time.monotonic()-start,artifact=name,sha256=sha(data)));save();print(label,result.returncode,flush=True)
 if result.returncode:raise RuntimeError(label+' failed')
save()
try:
 with tempfile.TemporaryDirectory(prefix='statechronicle-builtin-qualification-') as temporary:
  source=pathlib.Path(temporary)
  with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
  m['source_sha256']={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()};save()
  common=['--workspace','--all-features','--locked','--offline']
  check('fmt',['cargo','fmt','--all','--','--check'],source)
  check('finite-tests',['cargo','test',*common,'--exclude','statechronicle-fuzz','--all-targets'],source)
  check('doctests',['cargo','test',*common,'--exclude','statechronicle-fuzz','--doc'],source)
  check('strict-clippy',['cargo','clippy','--keep-going',*common,'--all-targets','--','-D','warnings'],source)
  check('strict-rustdoc',['cargo','doc',*common,'--no-deps'],source,{'RUSTDOCFLAGS':'-D warnings'})
  if not all(sha((source/p).read_bytes())==h for p,h in m['source_sha256'].items()):raise RuntimeError('source drift')
  m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
 m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
