import pathlib,subprocess,json,gzip,hashlib,os
root=pathlib.Path.cwd();base=root/'docs/0.3/evidence/maintenance/outbox-preflight';out=base/'driver-f99c37d';out.mkdir(exist_ok=False)
env=dict(os.environ,PYTHONOPTIMIZE='1',RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8')
m=dict(runner_commit='f99c37d3561c63d6fc4c7251d1b13fc365d0acd1',checks=[],status='running')
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for label,expected,revisions,extra in [('positive',0,['c418e98b91720188158c531cd5c2c4b8ab71cbcf','79baccfdfae964f8007dc086aa8a2f0d25f022b8','f99c37d3561c63d6fc4c7251d1b13fc365d0acd1'],[]),('negative',1,['79baccfdfae964f8007dc086aa8a2f0d25f022b8','f99c37d3561c63d6fc4c7251d1b13fc365d0acd1'],['--negative-control-ignore-outbox-digest'])]:
  before=set(base.glob('2026*'));command=['python3','scripts/check_proof_service_parity.py','--fixture','outbox']
  for revision in revisions:command+=['--revision',revision]
  command+=extra
  result=subprocess.run(command,cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
  new=set(base.glob('2026*'))-before
  if len(new)!=1:raise RuntimeError('ambiguous child evidence')
  child=new.pop();data=gzip.compress(result.stdout,mtime=0);name=label+'.log.gz';(out/name).write_bytes(data)
  child_manifest=json.loads((child/'manifest.json').read_text());passed=result.returncode==expected and child_manifest['different_rows']==(0 if label=='positive' else 552)
  m['checks'].append(dict(label=label,command=command,environment={k:env[k] for k in ['PYTHONOPTIMIZE','RUSTC_WRAPPER','CARGO_BUILD_JOBS']},exit_code=result.returncode,expected_exit_code=expected,passed=passed,child_manifest=str((child/'manifest.json').relative_to(root)),artifact=name,sha256=hashlib.sha256(data).hexdigest()));save();print(label,result.returncode,passed,flush=True)
  if not passed:raise RuntimeError('driver expected outcome failed')
 m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:save()
