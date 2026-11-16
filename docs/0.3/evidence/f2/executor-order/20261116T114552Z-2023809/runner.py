#!/usr/bin/env python3
"""Compare exact legacy errors, effects and port traces across committed revisions."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import time
import tomllib

from refactor_compat import packages


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--revision', action='append', required=True)
    cli.add_argument('--fixture', choices=['proof_service', 'executor', 'settlement'], default='proof_service')
    cli.add_argument('--negative-control-remove-expiry', action='store_true')
    cli.add_argument('--negative-control-commit-on-refusal', action='store_true')
    args = cli.parse_args()
    if args.negative_control_commit_on_refusal and args.fixture != 'settlement':
        cli.error('completion control requires settlement fixture')
    if args.negative_control_remove_expiry and args.fixture != 'executor':
        cli.error('expiry control requires executor fixture')
    if len(args.revision) < 2:
        cli.error('supply at least two revisions')
    root = Path(__file__).resolve().parents[1]
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=root)
    def sha(data):
        return hashlib.sha256(data).hexdigest()
    revisions = [git('rev-parse', rev).decode().strip() for rev in args.revision]
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + f'-{os.getpid()}'
    run = root / f'target/{args.fixture}-parity' / stamp
    evidence = root / ('docs/0.3/evidence/f2/executor-order' if args.fixture != 'proof_service' else 'docs/0.3/evidence/maintenance/proof-service') / stamp
    evidence.mkdir(parents=True)
    env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8', CARGO_TARGET_DIR=str(root/'target/proof-service-parity/build'))
    for key in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']:
        env.pop(key, None)
    fixture = root/f'ci/characterization/{args.fixture}.rs'
    support = git('show', 'c418e98b91720188158c531cd5c2c4b8ab71cbcf:crates/statechronicle/examples/common/domains.rs') if args.fixture == 'executor' else None
    if support is not None:
        provenance=json.loads((root/'ci/compat-0.2/provenance.json').read_text())
        assert sha(support)==provenance['files']['crates/consumer/examples/common/domains.rs']['sha256']
    wire = root/'ci/wire-0.2/history.json'
    manifest = {'revisions': revisions, 'fixture_sha256':sha(fixture.read_bytes()), 'negative_control_remove_expiry':args.negative_control_remove_expiry, 'negative_control_commit_on_refusal':args.negative_control_commit_on_refusal, 'wire_sha256':sha(wire.read_bytes()),
                'rustc':subprocess.check_output(['rustc','-vV'],cwd=root,text=True), 'checks':[], 'lanes':[], 'status':'running',
                'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},
                'limitations':['Four compatibility state-proof methods; new expected-claim and absence/trade verification are separate.',
                               'Deterministic matrix over frozen original wire; not exhaustive input/property/concurrency qualification.',
                               'A dependency-patched original comparator must be identified separately from unmodified release execution.']}
    if support is not None:
        manifest['support_sha256'] = sha(support)
        manifest['limitations'] = ['Nine legacy executor methods; six templates; zero, single and pairwise injected faults.', 'Sentinel host and compatibility sinks; no real durable storage, canonical settlement or concurrency qualification.']
        (evidence/'domains.rs').write_bytes(support)
    if args.fixture == 'settlement':
        manifest['limitations'] = ['Eight legacy coordinator routes; six structures; zero, single and pairwise injected faults.', 'Fixed sentinel host, snapshots and compatibility sinks; no real durable storage or canonical settlement qualification.']
    (evidence/f'{args.fixture}.rs').write_bytes(fixture.read_bytes())
    (evidence/'runner.py').write_bytes(Path(__file__).read_bytes())
    def save():
        (evidence/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    def check(label, command, cwd):
        start=time.monotonic()
        result=subprocess.run(command,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=600)
        artifact=label+'.log.gz';data=gzip.compress(result.stdout,mtime=0)
        (evidence/artifact).write_bytes(data)
        manifest['checks'].append({'label':label,'command':command,'cwd':str(cwd),'exit_code':result.returncode,'seconds':time.monotonic()-start,'artifact':artifact,'sha256':sha(data)})
        save();print(label,result.returncode,flush=True)
        if result.returncode: raise RuntimeError(label+' failed')
        return result.stdout
    save()
    try:
        traces=[]
        for index, revision in enumerate(revisions):
            source=run/str(index)/'source';source.mkdir(parents=True)
            data=git('archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz')
            with tarfile.open(fileobj=io.BytesIO(data)) as archive: archive.extractall(source,filter='data')
            if args.negative_control_remove_expiry and index == len(revisions)-1:
                owner=source/'crates/statechronicle-executor/src/evaluation.rs'
                original=owner.read_text()
                needle='conflict::check_expiry(intent, (self.now)())?;'
                assert original.count(needle)==1
                owner.write_text(original.replace(needle,'let _ = (self.now)(); /* test-only negative control: expiry refusal removed */'))
                (evidence/'negative-control.json').write_text(json.dumps({'revision':revision,'path':str(owner.relative_to(source)),'original_sha256':sha(original.encode()),'mutated_sha256':sha(owner.read_bytes()),'removed':needle},indent=2)+'\n')
            if args.negative_control_commit_on_refusal and index == len(revisions)-1:
                owner=source/'crates/statechronicle-executor/src/batch_completion.rs'
                original=owner.read_text()
                needle='handle.rollback().await'
                assert original.count(needle)==1
                owner.write_text(original.replace(needle,'handle.commit().await'))
                (evidence/'negative-control.json').write_text(json.dumps({'revision':revision,'path':str(owner.relative_to(source)),'original_sha256':sha(original.encode()),'mutated_sha256':sha(owner.read_bytes()),'removed':needle,'replacement':'handle.commit().await'},indent=2)+'\n')
            hashes={str(p.relative_to(source)):sha(p.read_bytes()) for p in sorted(source.rglob('*')) if p.is_file()}
            consumer=source.parent/'consumer';(consumer/'src').mkdir(parents=True)
            (consumer/'src/main.rs').write_bytes(fixture.read_bytes())
            if support is not None:
                (consumer/'src/domains.rs').write_bytes(support)
            dependencies=tomllib.loads((source/'Cargo.toml').read_text())['workspace']['dependencies']
            ed=dependencies['ed25519-dalek']['version']
            (consumer/'Cargo.toml').write_text('[package]\nname="proof-service-characterization"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nstatechronicle={path="../source/crates/statechronicle"}\ntracing={version="0.1",default-features=false,features=["std"]}\nchrono="0.4"\nserde={version="1",features=["derive"]}\nasync-trait="0.1"\nserde_json="1"\ntokio={version="1",features=["rt"]}\ned25519-dalek="'+ed+'"\n')
            (consumer/'Cargo.lock').write_bytes((source/'Cargo.lock').read_bytes())
            check(f'{index}-resolve',['cargo','metadata','--offline','--format-version','1'],consumer)
            assert not packages(consumer/'Cargo.lock')-packages(source/'Cargo.lock'), 'third-party dependency drift'
            lock=(consumer/'Cargo.lock').read_bytes();(evidence/f'{index}.lock').write_bytes(lock)
            build=['cargo','build','--locked','--offline']
            if args.fixture != 'proof_service':
                build.append('--release')
            check(f'{index}-build',build,consumer)
            check(f'{index}-clippy',['cargo','clippy','--locked','--offline','--','-D','warnings'],consumer)
            binary=Path(env['CARGO_TARGET_DIR'])/('release' if args.fixture != 'proof_service' else 'debug')/'proof-service-characterization'
            raw=check(f'{index}-trace',[str(binary),str(wire)],consumer)
            trace=json.loads(raw);assert len(trace)=={'executor':23544,'settlement':10128,'proof_service':2352}[args.fixture]
            traces.append(trace)
            manifest['lanes'].append({'revision':revision,'archive_sha256':sha(data),'source_sha256':hashes,'consumer_lock_sha256':sha(lock),'trace_sha256':sha(raw),'rows':len(trace),'binary_sha256':sha(binary.read_bytes())})
            assert (consumer/'Cargo.lock').read_bytes()==lock
            assert all(sha((source/name).read_bytes())==value for name,value in hashes.items())
            save()
        differences=[{'lane':index,'case':actual.get('case',actual.get('name')),'expected':expected,'actual':actual}
                     for index,trace in enumerate(traces[1:],1) for expected,actual in zip(traces[0],trace) if actual!=expected]
        if differences:
            data=gzip.compress(json.dumps(differences,separators=(',',':')).encode(),mtime=0)
            (evidence/'differences.json.gz').write_bytes(data)
        manifest['different_rows']=len(differences)
        assert not differences, 'error or port-order trace mismatch'
        manifest['status']='passed'
    except BaseException as error:
        manifest['status']='failed_or_interrupted';manifest['failure']=repr(error);raise
    finally:
        manifest['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(evidence.iterdir()) if p.name!='manifest.json'}
        save()
    print(evidence.relative_to(root),flush=True)


if __name__=='__main__': main()
