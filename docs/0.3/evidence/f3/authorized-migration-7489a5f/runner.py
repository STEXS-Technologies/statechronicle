import pathlib,subprocess,tarfile,io,os,json,gzip,hashlib,tempfile,difflib
root=pathlib.Path.cwd();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
out=root/'docs/0.3/evidence/f3'/('authorized-migration-'+revision[:7]);out.mkdir(exist_ok=False)
def sha(b):return hashlib.sha256(b).hexdigest()
archive=subprocess.check_output(['git','archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo'])
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/migration-controls'))
m=dict(candidate_commit=revision,archive_sha256=sha(archive),checks=[],status='running',limitations=['Original corpus independently produced at c418e98b91720188158c531cd5c2c4b8ab71cbcf.','Host migration grant, fixed keys, in-memory restartable stores and one historical tip; no database crash/current-head freshness claim.','Isolated negative source edits never mutate production checkout.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for lane in ['positive','ignore-migration-grant','ignore-source-binding','discard-rebuilt-rows','reuse-source-checkpoint','reuse-historical-event-id']:
  with tempfile.TemporaryDirectory(prefix='statechronicle-migration-control-') as temporary:
   source=pathlib.Path(temporary)
   with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
   owner=source/'crates/statechronicle/tests/support/authorized_migration.rs';before=owner.read_text();text=before
   test='versioned_recovery::authorized::migration_requires_separate_grant_and_pinned_original_source';markers=[]
   if lane=='ignore-migration-grant':needle='if self.allowed\n';replacement='if true\n';markers=['revoked escaped migration policy']
   elif lane=='ignore-source-binding':needle='            && context.intent.inputs == self.expected\n';replacement='';markers=['missing_source escaped migration policy']
   elif lane=='discard-rebuilt-rows':needle='        data.rows.insert(row.resource_id.clone(), row.clone());';replacement='        // Deliberately discard rebuilt projections.';markers=['assertion `left == right` failed','ContentDigest']
   elif lane=='reuse-source-checkpoint':
    owner=source/'crates/statechronicle-index/src/rebuild/verified.rs';before=owner.read_text();text=before
    needle='&("statechronicle.verified-rebuild.v0", job, &self.digest)';replacement='&("statechronicle.verified-rebuild.v0", job)';markers=['assertion `left != right` failed','verified-rebuild:']
   elif lane=='reuse-historical-event-id':needle='"evt_migration_{}"';replacement='"evt_{:020}"';markers=['duplicate event id','in rebuild stream']
   if lane!='positive':
    if text.count(needle)!=1:raise RuntimeError('source guard mismatch '+lane)
    text=text.replace(needle,replacement);owner.write_text(text)
    if lane in ['discard-rebuilt-rows','reuse-source-checkpoint','reuse-historical-event-id']:test='versioned_recovery::authorized::original_and_migrated_histories_resume_at_every_bounded_checkpoint'
   command=['cargo','test','-p','statechronicle','--test','custom_domains','--locked','--offline']+(['versioned_recovery::authorized','--','--nocapture'] if lane=='positive' else [test,'--','--exact','--nocapture'])
   result=subprocess.run(command,cwd=source,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
   name=lane+'.log.gz';data=gzip.compress(result.stdout,mtime=0);(out/name).write_bytes(data);log=result.stdout.decode(errors='replace')
   expected=0 if lane=='positive' else 101
   passed=result.returncode==expected and ('test result: ok. 3 passed' in log if lane=='positive' else 'test result: FAILED. 0 passed; 1 failed' in log) and all(marker in log for marker in markers)
   if lane=='positive':passed=passed and all(marker in log for marker in ['source_events=4 interrupted_checkpoint_cases=18','source_events=5 interrupted_checkpoint_cases=26'])
   if lane!='positive':(out/(lane+'.patch')).write_text(''.join(difflib.unified_diff(before.splitlines(True),text.splitlines(True),fromfile='original/'+owner.name,tofile='control/'+owner.name)))
   m['checks'].append(dict(lane=lane,command=command,exit_code=result.returncode,expected_exit_code=expected,passed=passed,failure_markers=markers,artifact=name,sha256=sha(data),source_before_sha256=sha(before.encode()),source_after_sha256=sha(text.encode())));save();print(lane,result.returncode,passed,flush=True)
   if not passed:raise RuntimeError(lane+' failed expected outcome')
 m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:save()
