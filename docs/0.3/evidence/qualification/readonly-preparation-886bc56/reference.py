from pathlib import Path
import subprocess,tarfile,io,os,json,time,hashlib
root=Path.cwd();base=root/'target/readonly-preparation/reference';source=base/'source';source.mkdir(parents=True,exist_ok=True)
revision='bb661a0974fa82dbe5778fa519c298790de778aa'
archive=subprocess.check_output(['git','archive',revision]);tarfile.open(fileobj=io.BytesIO(archive)).extractall(source,filter='data')
fixture=(root/'crates/statechronicle/tests/support/readonly.rs').read_text()
fixture=fixture.replace('use statechronicle::mutation::{PreparationConfig, PreparationPorts, Preparer};','')
a=fixture.index('pub(crate) fn preparer(');b=fixture.index('pub(crate) fn request(',a);fixture=fixture[:a]+fixture[b:]
fixture+='''
struct PreparationConfig {
    profiles: ProfileRegistry,
    executor: SubjectId,
    now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    event_id_fn: Box<dyn Fn() -> EventId + Send + Sync>,
    intent_verifier: Arc<dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync>,
}
'''
parity=(root/'crates/statechronicle/tests/preparation_parity.rs').read_text()
a=parity.index('fn legacy(');b=parity.index('#[tokio::test]',a)
fixture+='\nuse statechronicle::executor::pipeline::{Executor, Ports};\n'
fixture+=parity[a:b].replace('fn legacy(', 'pub(crate) fn preparer(').replace('common::','super::common::')
# Config only has local test visibility and does not participate in baseline production.
fixture=fixture.replace('pub(crate) fn config(', 'fn config(')
(source/'crates/statechronicle/tests/support/readonly.rs').write_text(fixture)
test=(root/'crates/statechronicle/tests/readonly_preparation.rs').read_text().replace('mod fixture;','mod fixture;\nmod common;')
(source/'crates/statechronicle/tests/preparation_reference.rs').write_text(test)
command=['cargo','+1.85.0','test','-p','statechronicle','--test','preparation_reference','--locked','--offline']
overrides={'RUSTC_WRAPPER':'','CARGO_BUILD_JOBS':'8','CARGO_TARGET_DIR':str(base/'target')}
start=time.monotonic()
with (base/'reference.log').open('w') as f:rc=subprocess.run(command,cwd=source,env=dict(os.environ,**overrides),stdout=f,stderr=subprocess.STDOUT).returncode
record={'reference_commit':revision,'candidate_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'command':command,'environment':overrides,'exit_code':rc,'seconds':time.monotonic()-start,'overlays':{str(p.relative_to(source)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [source/'crates/statechronicle/tests/support/readonly.rs',source/'crates/statechronicle/tests/preparation_reference.rs']},'limitation':'Previous protected-preparation implementation, not the frozen original release; test-only local config and legacy builder replace the new read-only constructor. Production archive unchanged.'}
(base/'manifest.json').write_text(json.dumps(record,indent=2)+'\n');print(json.dumps(record),flush=True)
assert rc==0
