import subprocess,tempfile,tarfile,io,pathlib,hashlib,gzip,json,os
root=pathlib.Path.cwd(); out=pathlib.Path(__file__).resolve().parent/'process-lock-control';out.mkdir()
source=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
archive=subprocess.check_output(['git','archive',source,'Cargo.toml','Cargo.lock','crates','fuzz','tutorials'])
with tempfile.TemporaryDirectory(prefix='statechronicle-lock-negative-') as temp:
    path=pathlib.Path(temp)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar: tar.extractall(path,filter='data')
    ledger=path/'tutorials/durable-custom-ledger/src/ledger.rs'
    before=ledger.read_bytes(); needle=b'        lock.try_lock().map_err(unavailable)?;'
    assert before.count(needle)==1
    after=before.replace(needle,b'        // Negative control: OS locking deliberately removed.')
    ledger.write_bytes(after)
    command=['cargo','+1.100.0','test','--manifest-path',str(path/'tutorials/durable-custom-ledger/Cargo.toml'),'--locked','--offline','killed_writer_reopens_at_each_file_boundary_with_exactly_once_effects','--','--exact','tests::killed_writer_reopens_at_each_file_boundary_with_exactly_once_effects','--nocapture']
    env=os.environ.copy();env.update(RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(path/'target'))
    result=subprocess.run(command,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,env=env,timeout=600)
    data=result.stdout
    (out/'lock-negative.log.gz').write_bytes(gzip.compress(data,mtime=0))
    expected=result.returncode==101 and b'assertion failed: FileLedger::open' in data and b'1 failed' in data
    report=dict(source=source,archive_sha256=hashlib.sha256(archive).hexdigest(),original_ledger_sha256=hashlib.sha256(before).hexdigest(),mutated_ledger_sha256=hashlib.sha256(after).hexdigest(),mutation='Remove only lock.try_lock; all subprocess assertions unchanged',command=command,exit_code=result.returncode,expected_failure_observed=expected,log_sha256=hashlib.sha256(data).hexdigest())
    (out/'lock-negative.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2));assert expected,data.decode(errors='replace')
