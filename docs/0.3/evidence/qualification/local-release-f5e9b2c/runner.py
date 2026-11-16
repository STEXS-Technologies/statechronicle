import gzip,hashlib,json,os,pathlib,subprocess,tempfile,time
root=pathlib.Path.cwd();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
out=root/'docs/0.3/evidence/qualification'/('local-release-'+revision[:7]);out.mkdir(exist_ok=False)
source=pathlib.Path(tempfile.mkdtemp(prefix='statechronicle-local-release-'))/'source'
sha=lambda b:hashlib.sha256(b).hexdigest()
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/local-release-qualification'))
m={'candidate_commit':revision,'status':'running','command':['bash','scripts/run_release_checks.sh'],'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},'rustc':subprocess.check_output(['rustc','-vV'],text=True),'limitations':['Local release driver includes bounded fuzz smoke only, not extended campaigns.','No matched performance, package qualification, final API approval or full roadmap completion claim.']}
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save();start=time.monotonic()
try:
 subprocess.run(['git','worktree','add','--detach',str(source),revision],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 paths=subprocess.check_output(['git','ls-files','-z'],cwd=source).decode().split('\0');paths=[p for p in paths if p]
 def inputs():return {p:sha((source/p).read_bytes()) for p in paths}
 m['source_sha256']=inputs();save()
 log=out/'release.log'
 with log.open('wb') as stream:
  result=subprocess.run(m['command'],cwd=source,env=env,stdout=stream,stderr=subprocess.STDOUT)
 m['exit_code']=result.returncode;m['source_unchanged']=m['source_sha256']==inputs()
 # Tutorial/native probes produce independent exact-revision manifests in the worktree.
 generated=[]
 for p in (source/'docs/0.3/evidence').rglob('*'):
  if p.is_file() and str(p.relative_to(source)) not in m['source_sha256']:
   dest=out/'nested'/p.relative_to(source/'docs/0.3/evidence');dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(p.read_bytes());generated.append(str(dest.relative_to(out)))
 m['nested_artifacts']=generated
 compressed=gzip.compress(log.read_bytes(),mtime=0);(out/'release.log.gz').write_bytes(compressed);log.unlink()
 m['status']='passed' if result.returncode==0 and m['source_unchanged'] else 'failed'
except BaseException as error:m['status']='failed_or_interrupted';m['error']=repr(error);raise
finally:
 m['seconds']=time.monotonic()-start
 m['artifacts_sha256']={str(p.relative_to(out)):sha(p.read_bytes()) for p in out.rglob('*') if p.is_file() and p.name!='manifest.json'}
 save()
 if source.exists():subprocess.run(['git','worktree','remove','--force',str(source)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
print(m['status'],m.get('exit_code'),flush=True)
