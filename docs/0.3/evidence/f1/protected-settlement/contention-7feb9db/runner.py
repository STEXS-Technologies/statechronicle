import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile
root=pathlib.Path.cwd();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
out=root/'docs/0.3/evidence/f1/protected-settlement'/('contention-'+revision[:7]);out.mkdir(exist_ok=False)
def sha(b):return hashlib.sha256(b).hexdigest()
archive=subprocess.check_output(['git','archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','.cargo'])
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'target/settlement-controls'))
m=dict(candidate_commit=revision,archive_sha256=sha(archive),checks=[],status='running',limitations=['Fixed keys and all-allow authorizer are fixtures.','One mutex-backed in-memory authority, fresh handles and real 2/8/32 OS-thread contenders; no database process/power-loss claim.','Deliberate negative controls mutate isolated archived source only.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
save()
try:
 for lane in ['positive','accept-stale-authority','leak-payment-projection','acknowledge-lost-response']:
  with tempfile.TemporaryDirectory(prefix='statechronicle-economic-') as tmp:
   source=pathlib.Path(tmp)
   with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
   owner=source/'crates/statechronicle/tests/support/durable.rs';before=owner.read_text();text=before
   test=None
   if lane=='accept-stale-authority':
    for error in ['canonical head changed','projected state changed','prepared state root mismatch']:
     pos=text.index('"'+error+'"');start=text.rfind('if ',0,pos);end=text.rfind('{',start,pos)
     if start<0 or end<start:raise RuntimeError('guard missing: '+error)
     condition=text[start+3:end].strip();text=text[:start]+'if false && ('+condition+') '+text[end:]
    test='contenders::distinct_purchases_have_one_atomic_winner_and_recover_original_losers'
   elif lane=='leak-payment-projection':
    needle='        self.ledger.checkpoint(Step::Projection).await?;'
    if text.count(needle)!=1:raise RuntimeError('projection boundary missing')
    text=text.replace(needle,'''        if self.ledger.fail_at == Some((Step::Projection, 2))
            && projection.state_type == statechronicle::StateType::FungibleBalance {
            self.ledger.data.lock().unwrap().projections.push(projection.clone());
        }
'''+needle)
    test='contenders::economic_group_failure_preserves_every_seeded_effect_and_original_identity'
   elif lane=='acknowledge-lost-response':
    needle='''        if self.ledger.lose_commit_response {
            return Err(LedgerStoreError::Unavailable(String::from(
                "commit response lost",
            )));
        }'''
    if text.count(needle)!=1:raise RuntimeError('ack boundary missing')
    text=text.replace(needle,'        if self.ledger.lose_commit_response { return Ok(()); }')
    test='contenders::economic_group_lost_ack_recovers_original_with_current_access_checks'
   owner.write_text(text)
   command=['cargo','test','-p','statechronicle','--test','protected_settlement','--locked','--offline']+([test,'--','--exact','--nocapture'] if test else ['--','--nocapture'])
   result=subprocess.run(command,cwd=source,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
   name=lane+'.log.gz';data=gzip.compress(result.stdout,mtime=0);(out/name).write_bytes(data)
   log=result.stdout.decode(errors='replace');expected=0 if lane=='positive' else 101
   passed=result.returncode==expected and ('test result: ok. 9 passed' in log if lane=='positive' else 'test result: FAILED. 0 passed; 1 failed' in log)
   m['checks'].append(dict(lane=lane,command=command,exit_code=result.returncode,expected_exit_code=expected,passed=passed,artifact=name,sha256=sha(data),source_before_sha256=sha(before.encode()),source_after_sha256=sha(text.encode())))
   if lane!='positive':
    import difflib
    patch=''.join(difflib.unified_diff(before.splitlines(True),text.splitlines(True),fromfile='original/durable.rs',tofile='control/durable.rs'))
    (out/(lane+'.patch')).write_text(patch)
   save();print(lane,result.returncode,passed,flush=True)
   if not passed:raise RuntimeError(lane+' did not meet expected result')
 m['status']='passed'
except BaseException as e:m['status']='failed_or_interrupted';m['failure']=repr(e);raise
finally:save()
