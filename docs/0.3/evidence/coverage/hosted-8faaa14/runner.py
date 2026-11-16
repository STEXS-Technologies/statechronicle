import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();out=root/'docs/0.3/evidence/coverage'/('hosted-'+revision[:7]);out.mkdir()
def sha(b):return hashlib.sha256(b).hexdigest()
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','.cargo','crates','fuzz','scripts','ci/coverage-baseline.json'];archive=subprocess.check_output(['git','archive',revision,'--',*paths]);env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8')
m={'candidate':revision,'archive_paths':paths,'archive_sha256':sha(archive),'rustc':subprocess.check_output(['rustc','-vV'],text=True),'cargo_llvm_cov':subprocess.check_output(['cargo','llvm-cov','--version'],text=True),'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS']},'checks':[],'status':'running'};(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,work):
 start=time.monotonic();r=subprocess.run(command,cwd=work,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=1800);data=gzip.compress(r.stdout,mtime=0);name=label+'.log.gz';(out/name).write_bytes(data);m['checks'].append({'label':label,'command':command,'exit_code':r.returncode,'seconds':time.monotonic()-start,'artifact':name,'sha256':sha(data)});save();print(label,r.returncode,flush=True)
 if r.returncode:raise RuntimeError(label+' failed')
save()
try:
 with tempfile.TemporaryDirectory(prefix='statechronicle-coverage-') as temporary:
  work=pathlib.Path(temporary)
  with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(work,filter='data')
  m['source_sha256']={str(p.relative_to(work)):sha(p.read_bytes()) for p in work.rglob('*') if p.is_file()};save()
  check('collection',['bash','scripts/run_coverage_json.sh'],work)
  for name in ['coverage-full.json','coverage.json']:
   data=(work/'target'/name).read_bytes();(out/name).write_bytes(data);report=json.loads(data);count=sum(d['totals']['lines']['count'] for d in report['data']);covered=sum(d['totals']['lines']['covered'] for d in report['data']);m.setdefault('reports',{})[name]={'count':count,'covered':covered,'percent':100*covered/count,'sha256':sha(data)}
  for source in ['scripts/run_coverage_json.sh','scripts/check_coverage_ratchet.sh','scripts/test_coverage_ratchet.py','ci/coverage-baseline.json']:(out/pathlib.Path(source).name).write_bytes((work/source).read_bytes())
  save();check('ratchet',['bash','scripts/check_coverage_ratchet.sh'],work)
  if any(sha((work/p).read_bytes())!=h for p,h in m['source_sha256'].items()):raise RuntimeError('source or lock drift')
  m['status']='passed'
except BaseException as error:m['status']='failed_or_interrupted';m['error']=repr(error);raise
finally:save()
print(out.relative_to(root),flush=True)
