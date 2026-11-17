"""Check final conformance runtime controls and the file-lock bypass reproducer."""
import gzip,hashlib,io,json,subprocess,tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;sha=lambda b:hashlib.sha256(b).hexdigest()
base=root/'docs/0.3/evidence/conformance/concurrent-mixed-0799afe'
subprocess.run(['python3',str(base/'verify-checkpoint.py')],check=True)
# Verify the file control against the exact one-line mutation, with actual fresh compilation.
base=out/'process-lock-control';m=json.loads((base/'lock-negative.json').read_text());archive=subprocess.check_output(['git','archive',m['source'],'Cargo.toml','Cargo.lock','crates','fuzz','tutorials'])
assert sha(archive)==m['archive_sha256']
with tarfile.open(fileobj=io.BytesIO(archive)) as handle:before=handle.extractfile('tutorials/durable-custom-ledger/src/ledger.rs').read()
needle=b'        lock.try_lock().map_err(unavailable)?;';assert before.count(needle)==1
after=before.replace(needle,b'        // Negative control: OS locking deliberately removed.')
assert sha(before)==m['original_ledger_sha256'] and sha(after)==m['mutated_ledger_sha256']
raw=gzip.decompress((base/'lock-negative.log.gz').read_bytes());assert sha(raw)==m['log_sha256']
assert m['exit_code']==101 and m['expected_failure_observed'] and b'assertion failed: FileLedger::open' in raw and b'1 failed' in raw
assert b'error[E' not in raw and b'Compiling statechronicle-durable-tutorial' in raw
assert b'tests::killed_writer_reopens_at_each_file_boundary_with_exactly_once_effects ... FAILED' in raw
for crate in (root/'crates').iterdir():
    if (crate/'Cargo.toml').exists():assert ('Compiling '+crate.name+' v').encode() in raw
result=dict(status='passed',file_lock_control_manifest_sha256=sha((base/'lock-negative.json').read_bytes()),conformance_checkpoint_sha256=sha((root/'docs/0.3/evidence/conformance/concurrent-mixed-0799afe/checkpoint.json').read_bytes()),limitations=['Bounded reference schedules and selected actual process terminations, not arbitrary backend or power-loss certification.','Original ineffective deadlock mutation retains its failed campaign status; corrected mutation is a runtime rejection.'])
(out/'control-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
