"""Build or measure identical external canonical API consumers on two frozen lanes."""
import argparse, gzip, hashlib, io, json, os
from pathlib import Path
import platform, shutil, statistics, subprocess, tarfile, time, tomllib

cli=argparse.ArgumentParser(description=__doc__)
cli.add_argument('--measure', action='store_true')
args=cli.parse_args()
root=Path.cwd(); driver=Path(__file__).resolve(); out=driver.parent/'canonical-performance'
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
    m['checks'].append(dict(label=label,command=command,exit_code=p.returncode,seconds=time.monotonic()-started,artifact=name,sha256=sha(compressed)))
    save()
    if p.returncode:
        raise RuntimeError(label+' failed: '+str(out/name))
    return p.stdout
if not args.measure:
    out.mkdir(exist_ok=False)
    candidate=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
    m=dict(candidate_commit=candidate, control_commit='a889f172a65bf203a0dde989d372dd111ae4b73e',status='building',checks=[],lanes={},cpu=min(os.sched_getaffinity(0)),
        compiler=subprocess.check_output(['rustc','-vV'],text=True),limitations=['Synchronous in-memory fake; no physical backend throughput claim.',
        'Identical consumer fixture and request across lanes, including assertions.', 'Current canonical APIs have no equivalent 0.2 authority contract; control is the earlier coherent 0.3 implementation.',
        'One thousand individual action times per workload per cycle; seven alternated cycles; shared host CPU affinity.',
        'Concurrency workload includes creation/join of eight threads and exact-identity contention; not general adapter scaling.'])
    (out/'runner.py').write_bytes(driver.read_bytes());(out/'benchmark.rs').write_bytes(driver.with_name('canonical_bench.rs').read_bytes())
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
        binary=out/('bench-'+lane);shutil.copy2(Path(env['CARGO_TARGET_DIR'])/'release/statechronicle-final-consumer',binary)
        assert all(sha((source/p).read_bytes())==h for p,h in hashes.items())
        m['lanes'][lane]=dict(revision=revision,archive_paths=paths,archive_sha256=sha(data),source_sha256=hashes,binary_sha256=sha(binary.read_bytes()),binary_bytes=binary.stat().st_size,consumer_lock_sha256=sha((consumer/'Cargo.lock').read_bytes()),consumer_sha256={str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()})
        save();print(lane,'built',flush=True)
    m['status']='built';save()
else:
    m=json.loads((out/'manifest.json').read_text());assert m['status']=='built'
    def conditions():return dict(load_average=os.getloadavg(),utc=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),kernel=platform.release(),cpu=m['cpu'])
    m['conditions_before']=conditions();m['status']='measuring';save();values={}
    try:
        for cycle in range(7):
            for lane in (['control','candidate'] if cycle%2==0 else ['candidate','control']):
                binary=out/('bench-'+lane);assert sha(binary.read_bytes())==m['lanes'][lane]['binary_sha256']
                result=json.loads(check(str(cycle)+'-'+lane,['taskset','-c',str(m['cpu']),str(binary)],root))
                for record in result['workloads']:values.setdefault(record['workload'],{}).setdefault(lane,[]).append(record)
            print('canonical cycle',cycle,'complete',flush=True)
        summary=[]
        for name, lanes in values.items():
            ratios={metric:[(b[metric]/a[metric]-1)*100 for a,b in zip(lanes['control'],lanes['candidate'])] for metric in ['median_ns','p95_ns','p99_ns']}
            summary.append(dict(workload=name,observations=lanes,paired_percent_deltas=ratios,median_paired_percent_delta={k:statistics.median(v) for k,v in ratios.items()}))
        (out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
        allocation={}
        for lane in ['control','candidate']:
            allocation[lane]={}
            binary=out/('bench-'+lane)
            for name in values:
                raw=check('allocation-'+lane+'-'+name,['taskset','-c',str(m['cpu']),'valgrind','--tool=memcheck','--leak-check=no','--error-exitcode=97',str(binary),'alloc-'+name],root)
                import re
                match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);assert match
                allocation[lane][name]=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]))
            control=allocation[lane]['control']
            for totals in allocation[lane].values():totals['control_subtracted_per_action']={k:(totals[k]-control[k])/1000 for k in ['allocations','bytes']}
        m['allocations']=allocation;m['status']='passed'
    except BaseException as error:m.update(status='failed',error=repr(error));raise
    finally:m['conditions_after']=conditions();save()
print(m['status'],flush=True)
