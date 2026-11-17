"""Build or measure signed-history authentication and resumed rebuild on frozen lanes."""
import argparse, gzip, hashlib, io, json, os
from pathlib import Path
import platform, shutil, statistics, subprocess, tarfile, time, tomllib

cli=argparse.ArgumentParser(description=__doc__)
cli.add_argument('--measure', action='store_true')
args=cli.parse_args()
root=Path.cwd(); driver=Path(__file__).resolve(); out=driver.parent/'history-performance'
sha=lambda b:hashlib.sha256(b).hexdigest()
env=dict(os.environ,RUSTC_WRAPPER='',CARGO_BUILD_JOBS='8')
for key in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTDOCFLAGS']:
    env.pop(key,None)
def save():
    (out/'manifest.json').write_text(json.dumps(m,indent=2)+'\n')
def check(label, command, cwd):
    started=time.monotonic()
    p=subprocess.run(command,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=900)
    compressed=gzip.compress(p.stdout,mtime=0);name=label+'.log.gz';(out/name).write_bytes(compressed)
    m['checks'].append(dict(label=label,command=command,exit_code=p.returncode,seconds=time.monotonic()-started,artifact=name,sha256=sha(compressed),extra_environment={k:env[k] for k in ['STATECHRONICLE_ALLOCATION_PROBE','STATECHRONICLE_ALLOCATION_ACTIONS'] if k in env}))
    save()
    if p.returncode:
        raise RuntimeError(label+' failed: '+str(out/name))
    return p.stdout
if not args.measure and out.exists():
    prior=json.loads((out/'manifest.json').read_text())
    assert prior['status']=='built','Existing history build is not a reusable successful build.'
    assert (out/'runner.py').read_bytes()==driver.read_bytes()
    assert (out/'benchmark.rs').read_bytes()==driver.with_name('history_bench.rs').read_bytes()
    for lane,item in prior['lanes'].items():
        assert sha((out/('bench-'+lane.ljust(9,'_'))).read_bytes())==item['binary_sha256']
    print('Reusing the already completed fresh, hash-checked signed-history build.',flush=True)
    raise SystemExit(0)
if not args.measure:
    out.mkdir(exist_ok=False)
    candidate=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
    m=dict(candidate_commit=candidate, control_commit='a889f172a65bf203a0dde989d372dd111ae4b73e',status='building',checks=[],lanes={},cpu=min(os.sched_getaffinity(0)),
        compiler=subprocess.check_output(['rustc','-vV'],text=True),limitations=['Synchronous in-memory fake; no physical backend throughput claim.',
        'Identical consumer fixture and request across lanes, including assertions.', 'Current canonical APIs have no equivalent 0.2 authority contract; control is the earlier coherent 0.3 implementation.',
        'Seven alternated cycles: 100 individual samples for one-event history and 31 for 32/256-event histories; 1000 timer controls; three warmups.',
        'Complete signed histories of 1,32,256 commits; verification and already-verified checkpoint/rebuild paths measured separately. Memory sink/fresh handles and injected checkpoint ACK loss do not prove physical durability.'])
    (out/'runner.py').write_bytes(driver.read_bytes());(out/'benchmark.rs').write_bytes(driver.with_name('history_bench.rs').read_bytes())
    fixture_paths=['crates/statechronicle/tests/common/mod.rs','crates/statechronicle/tests/common/harness.rs','crates/statechronicle/tests/support/durable.rs','crates/statechronicle/tests/fixtures/history.json']
    fixture={p:subprocess.check_output(['git','show',candidate+':'+p]) for p in fixture_paths}
    m['fixture_sha256']={p:sha(b) for p,b in fixture.items()};save()
    work=root/'target/final-canonical-performance'/(candidate[:7]+'-'+str(os.getpid()));work.mkdir(parents=True,exist_ok=False)
    for lane, revision in [('control',m['control_commit']),('candidate',candidate)]:
        source=work/lane/'source';source.mkdir(parents=True)
        paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz']
        data=subprocess.check_output(['git','archive',revision,'--',*paths])
        with tarfile.open(fileobj=io.BytesIO(data)) as tar:tar.extractall(source,filter='data')
        consumer=work/lane/'consumer';(consumer/'src/common').mkdir(parents=True)
        (consumer/'src/main.rs').write_bytes(b'#![allow(dead_code, unused_variables, unused_imports)]\n'+(out/'benchmark.rs').read_bytes())
        for dest, original in [('common/mod.rs',fixture_paths[0]),('common/harness.rs',fixture_paths[1]),('ledger.rs',fixture_paths[2]),('history.json',fixture_paths[3])]:
            (consumer/'src'/dest).write_bytes(fixture[original])
        cargo=subprocess.check_output(['git','show',candidate+':ci/compat-0.2/crates/consumer/Cargo.toml']).decode()
        cargo=cargo.replace('statechronicle-legacy-consumer','statechronicle-final-consumer').replace('../../../../crates/statechronicle',str(source/'crates/statechronicle'))
        (consumer/'Cargo.toml').write_text(cargo);shutil.copyfile(source/'Cargo.lock',consumer/'Cargo.lock')
        hashes={str(p.relative_to(source)):sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
        env['CARGO_TARGET_DIR']=str(work/lane/'target')
        check(lane+'-resolve',['cargo','metadata','--offline','--format-version','1'],consumer)
        def packages(p):return {(i['name'],i['version'],i.get('source'),i.get('checksum')) for i in tomllib.loads(p.read_text())['package'] if i.get('source')}
        assert not packages(consumer/'Cargo.lock')-packages(source/'Cargo.lock')
        check(lane+'-build',['cargo','build','--release','--locked','--offline'],consumer)
        binary=out/('bench-'+lane.ljust(9,'_'));shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/statechronicle-final-consumer',binary)
        assert all(sha((source/p).read_bytes())==h for p,h in hashes.items())
        m['lanes'][lane]=dict(revision=revision,archive_paths=paths,archive_sha256=sha(data),source_sha256=hashes,binary_sha256=sha(binary.read_bytes()),binary_bytes=binary.stat().st_size,consumer_lock_sha256=sha((consumer/'Cargo.lock').read_bytes()),consumer_sha256={str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()})
        save();print(lane,'built',flush=True)
    m['status']='built';save()
else:
    m=json.loads((out/'manifest.json').read_text());assert m['status']=='built'
    def conditions():return dict(load_average=os.getloadavg(),utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),kernel=platform.release(),cpu=m['cpu'])
    m['conditions_before']=conditions();m['status']='measuring';m['allocation_limitations']=['Process totals include identical fixed generation/verification of all three fixture sizes.','Equal-length lane executable names and 128-byte arguments; trim truncates without reallocation.','Matched controls use the same action-count environment. Process exit skips destruction, so frees are not leak qualification.','1000,100,10 actions for 1,32,256 events respectively; control-subtracted counts describe this fixture, not arbitrary backend memory.'];save();values={}
    try:
        for cycle in range(7):
            for lane in (['control','candidate'] if cycle%2==0 else ['candidate','control']):
                binary=out/('bench-'+lane.ljust(9,'_'));assert sha(binary.read_bytes())==m['lanes'][lane]['binary_sha256']
                result=json.loads(check(str(cycle)+'-'+lane,['taskset','-c',str(m['cpu']),str(binary)],root))
                if 'observed_fixture_digests' not in m:m['observed_fixture_digests']=result['fixture_digests'];save()
                assert result['fixture_digests']==m['observed_fixture_digests']
                for record in result['workloads']:values.setdefault(record['workload'],{}).setdefault(lane,[]).append(record)
            print('history cycle',cycle,'complete',flush=True)
        summary=[]
        for name, lanes in values.items():
            ratios={metric:[(b[metric]/a[metric]-1)*100 for a,b in zip(lanes['control'],lanes['candidate'])] for metric in ['median_ns','p95_ns','p99_ns']}
            summary.append(dict(workload=name,observations=lanes,paired_percent_deltas=ratios,median_paired_percent_delta={k:statistics.median(v) for k,v in ratios.items()}))
        (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
        allocation={}
        for lane in ['control','candidate']:
            allocation[lane]={};binary=out/('bench-'+lane.ljust(9,'_'))
            assert sha(binary.read_bytes())==m['lanes'][lane]['binary_sha256']
            probes=[('control',n) for n in [1000,100,10]]+[(name,1000 if name.endswith('_1') else 100 if name.endswith('_32') else 10) for name in values if name!='control']
            for name,count in probes:
                env.update(STATECHRONICLE_ALLOCATION_PROBE='1',STATECHRONICLE_ALLOCATION_ACTIONS=str(count))
                raw=check('allocation-'+lane+'-'+name+'-'+str(count),['taskset','-c',str(m['cpu']),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),name.ljust(128,'_')],root)
                import re
                assert re.search(rb'ERROR SUMMARY: 0 errors',raw)
                match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
                allocation[lane][name+'@'+str(count)]=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]))
                allocation[lane][name+'@'+str(count)]['actions']=count
            for name,totals in allocation[lane].items():
                count=totals['actions'];control=allocation[lane]['control@'+str(count)]
                totals['control_subtracted_per_action']={key:(totals[key]-control[key])/count for key in ['allocations','bytes']}
        m['allocations']=allocation;m['status']='passed'
    except BaseException as error:m.update(status='failed',error=repr(error));raise
    finally:m['conditions_after']=conditions();save()
print(m['status'],flush=True)
