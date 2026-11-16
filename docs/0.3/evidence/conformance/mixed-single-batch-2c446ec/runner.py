"""Run from repository root; qualify the immutable mixed-kind history source."""
import gzip,hashlib,io,json,os,pathlib,subprocess,tarfile,tempfile,time
root=pathlib.Path.cwd()
revision='2c446ec6b1cfb8c4ce4378e4f3ff40f995cfb33f'
previous='a04c70c2b4fbed7b7f8c1770925ffd33cdf73daf'
out=root/'docs/0.3/evidence/conformance/mixed-single-batch-2c446ec'
out.mkdir(parents=True,exist_ok=False)
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz','ci/conformance']
archive=subprocess.check_output(['git','archive',revision,'--',*paths])
def sha(data):return hashlib.sha256(data).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],text=True).strip()
assert not git('diff',revision,'--',*paths)
changed=git('diff','--name-only',previous,revision).splitlines()
assert all(p.startswith('ci/conformance/') or p=='docs/0.3/adapter-contract.md' for p in changed)
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8',CARGO_TARGET_DIR=str(root/'ci/conformance/target'))
m=dict(candidate_commit=revision,previous_commit=previous,changed_source_files=changed,archive_paths=paths,archive_sha256=sha(archive),rustc=subprocess.check_output(['rustc','+1.100.0','-vV'],text=True),cargo=subprocess.check_output(['cargo','+1.100.0','-V'],text=True),environment={k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},checks=[],status='running',limitations=['Bounded deterministic logical reference backend, not randomized concurrency or physical crash safety.','Independent fixture signed-history checks do not qualify production authentication or historical signer policy.','Production library, dependencies, locks and workflow files identical to previous source; no new root workspace tests claimed.'])
(out/'runner.py').write_bytes(pathlib.Path(__file__).read_bytes())
def save():(out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label,command,cwd,extra=None):
    start=time.monotonic()
    try:
        result=subprocess.run(command,cwd=cwd,env=dict(env,**(extra or {})),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180)
        data=result.stdout;code=result.returncode
    except subprocess.TimeoutExpired as error:
        data=error.stdout or b'';code='timeout'
    artifact=label+'.log.gz';compressed=gzip.compress(data,mtime=0);(out/artifact).write_bytes(compressed)
    m['checks'].append(dict(label=label,command=command,exit_code=code,seconds=time.monotonic()-start,artifact=artifact,sha256=sha(compressed),extra_environment=extra or {}));save()
    print(label,code,flush=True)
    assert code==0,(label,data.decode(errors='replace'))
    if label=='tests': assert b'22 passed; 0 failed;' in data
save()
try:
    with tempfile.TemporaryDirectory(prefix='statechronicle-mixed-histories-') as temporary:
        source=pathlib.Path(temporary)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(source,filter='data')
        m['source_sha256']={str(p.relative_to(source)):sha(p.read_bytes()) for p in sorted(source.rglob('*')) if p.is_file()};save()
        manifest=['--manifest-path','ci/conformance/Cargo.toml']
        locked=['--all-features','--locked','--offline']
        check('fmt',['cargo','+1.100.0','fmt',*manifest,'--package','statechronicle-conformance','--','--check'],source)
        check('tests',['cargo','+1.100.0','test',*manifest,'--all-targets',*locked,'--','--nocapture'],source)
        check('doctests',['cargo','+1.100.0','test',*manifest,'--doc',*locked],source)
        check('strict-clippy',['cargo','+1.100.0','clippy',*manifest,'--all-targets',*locked,'--','-D','warnings'],source)
        check('strict-rustdoc',['cargo','+1.100.0','doc',*manifest,'--no-deps',*locked],source,{'RUSTDOCFLAGS':'-D warnings'})
        assert all(sha((source/p).read_bytes())==h for p,h in m['source_sha256'].items())
        assert not git('diff',revision,'--',*paths)
        m['status']='passed'
except BaseException as error:
    m['status']='failed_or_interrupted';m['failure']=repr(error);raise
finally:
    m['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in out.iterdir() if p.name!='manifest.json'};save()
print(out.relative_to(root),flush=True)
