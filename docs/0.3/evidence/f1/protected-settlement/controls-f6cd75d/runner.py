import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd(); revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
out=root/'docs/0.3/evidence/f1/protected-settlement'/('controls-'+revision[:7]);out.mkdir(exist_ok=False)
def sha(data):return hashlib.sha256(data).hexdigest()
archive=subprocess.check_output(['git','archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo'])
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/settlement-controls'))
m=dict(candidate_commit=revision,archive_sha256=sha(archive),checks=[],status='running',limitations=['Deliberate bypasses in isolated source copies only; no production mutation.','Bounded preparation regression checks, not canonical economic contention or physical storage qualification.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for lane in ['positive','skip-settlement-validation','skip-bundle-routing']:
  with tempfile.TemporaryDirectory(prefix='statechronicle-settlement-control-') as temp:
   source=pathlib.Path(temp)
   with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
   test=None;needle=None;owner=None
   if lane=='skip-settlement-validation':
    owner=source/'crates/statechronicle-executor/src/evaluation.rs';text=owner.read_text();a=text.index('        if settlement {\n');b=text.index('        Ok(prepared)',a);needle=text[a:b];owner.write_text(text[:a]+text[b:]);test='protected_settlement_refuses_incomplete_or_mismatched_groups_without_writes'
   elif lane=='skip-bundle-routing':
    owner=source/'crates/statechronicle-profiles/src/resource.rs';text=owner.read_text();a=text.index('        if !coordinated\n            && intent.operation == *asset_op::trade_settle()\n            && atomicity::declares_bundle(intent)');b=text.index('        Ok(())',a);needle=text[a:b];owner.write_text(text[:a]+text[b:]);test='incomplete_bundle_cannot_escape_single_or_batch_protected_preparation'
   command=['cargo','test','-p','statechronicle','--test','protected_settlement','--locked','--offline']+([test,'--','--exact'] if test else [])
   result=subprocess.run(command,cwd=source,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
   name=lane+'.log.gz';data=gzip.compress(result.stdout,mtime=0);(out/name).write_bytes(data)
   text=result.stdout.decode(errors='replace');expected=0 if lane=='positive' else 101
   passed=result.returncode==expected and ('test result: ok. 5 passed' in text if lane=='positive' else 'test result: FAILED. 0 passed; 1 failed' in text)
   if lane=='skip-settlement-validation':passed=passed and 'missing_payment escaped' in text
   if lane=='skip-bundle-routing':passed=passed and 'incomplete bundle escaped' in text
   m['checks'].append(dict(lane=lane,command=command,exit_code=result.returncode,expected_exit_code=expected,passed=passed,artifact=name,sha256=sha(data),removed_source=needle,mutated_source_sha256=sha(owner.read_bytes()) if owner else None));save();print(lane,result.returncode,passed,flush=True)
   if not passed:raise RuntimeError(lane+' failed qualification')
 m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:save()
