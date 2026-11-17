"""Verify terminal performance logs, source archives, binaries and summary math."""
import gzip, hashlib, io, json, math, re, statistics, subprocess, tarfile
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent
sha=lambda b:hashlib.sha256(b).hexdigest()
paths=['Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz']
crates=[p.parent.name for p in (root/'crates').glob('*/Cargo.toml')]
def load(p):return json.loads(p.read_text())
def require(ok,msg):
    if not ok:raise ValueError(msg)
def archive(revision,selected):
    data=subprocess.check_output(['git','archive',revision,'--',*selected])
    with tarfile.open(fileobj=io.BytesIO(data)) as handle:files={p.name:handle.extractfile(p).read() for p in handle if p.isfile()}
    return data,files
def checked_manifest(base):
    m=load(base/'manifest.json');require(m['status']=='passed',str(base)+' terminal pass')
    logs={}
    for c in m['checks']:
        require(c['exit_code']==0,'exit '+str(c))
        data=(base/c['artifact']).read_bytes();require(sha(data)==c['sha256'],'check hash '+c['artifact']);logs[c.get('label',c['artifact'])]=gzip.decompress(data)
    for name,value in m.get('artifacts_sha256',{}).items():require(sha((base/name).read_bytes())==value,'artifact '+name)
    return m,logs
def sample_math(record):
    values=record.get('samples_ns_per_operation',record.get('samples_ns'));n=len(values)
    require(n>=31 and values==sorted(values) and all(math.isfinite(v) and v>=0 for v in values),'sample sequence')
    require(record['median_ns']==values[n//2],'median')
    for p in [95,99]:require(record[f'p{p}_ns']==values[math.ceil(n*p/100)-1],f'p{p}')
    if record.get('timing_kind')=='individual action':require(record['sample_count']==n and record['iterations_per_sample']==1,'individual count')
def source_lanes(base,m,logs,matrix):
    stamp=base.name.removeprefix('matrix-').removeprefix('per-request-').removeprefix('all-paths-')
    run=root/('target/performance-matrix' if base.name.startswith('matrix-') else 'target/per-request')/stamp
    for index,(lane,item) in enumerate(m['lanes'].items()):
        original=matrix['lanes'][lane];data,files=archive(original['revision'],paths)
        require(sha(data)==original['archive_sha256'],'source archive '+lane)
        for patch in original['manifest_patches']:
            p=patch['path'];require(sha(files[p])==patch['before_sha256'],'original patch input')
            replacement=(root/matrix['_path']/('dependency-control-'+p.replace('/','_'))).read_bytes()
            require(sha(replacement)==patch['after_sha256'],'patch replacement');files[p]=replacement
        require({p:sha(b) for p,b in files.items()}==original['source_sha256'],'source map '+lane)
        source=run/lane/'source';require(all(sha((source/p).read_bytes())==sha(b) for p,b in files.items()),'retained source '+lane)
        code=(base/(lane+'-benchmark.rs')).read_bytes();require(sha(code)==item['benchmark_sha256'],'benchmark '+lane)
        binary=run/f'lane-{index}-bench';require(sha(binary.read_bytes())==item['binary_sha256'],'binary '+lane)
        require(item['lock_sha256']==original['lock_sha256'],'lane lock')
        build=logs[lane+'-build']
        for crate in crates:require(re.search(rb'Compiling '+crate.encode()+rb' v',build),'fresh '+lane+' '+crate)
    require(len({i['benchmark_sha256'] for i in m['lanes'].values()})==1,'same consumer')
def observations(base,m,logs,expected,matrix_style=False):
    summary=load(base/'summary.json');require(len(summary)==expected,'workload count '+str(base));seen=set()
    for record in summary:
        name=record['workload'];regime=record.get('regime','individual');require((regime,name) not in seen,'unique workload');seen.add((regime,name))
        for lane,observed in record['observations'].items():
            require(len(observed)==7,'seven cycles '+name)
            for cycle,value in enumerate(observed):
                sample_math(value)
                if matrix_style:label=f'default-{cycle}-{lane}' if regime=='default' else f'extended-{cycle}-{lane}-{name}'
                else:label=f'cycle-{cycle}-{lane}-{name}'
                raw=json.loads(logs[label])['workloads'];require([v for v in raw if v['workload']==name]==[value],'raw observation '+label)
        for pair,comp in record['comparisons'].items():
            control,candidate=pair.split(' -> ')
            for metric,deltas in comp['paired_percent_deltas'].items():
                expected_deltas=[(b[metric]/a[metric]-1)*100 if a[metric] else None for a,b in zip(record['observations'][control],record['observations'][candidate])]
                require(deltas==expected_deltas,'paired deltas '+name)
                if any(v is not None for v in deltas):require(comp['median_paired_percent_deltas'][metric]==statistics.median(v for v in deltas if v is not None),'paired median')
    return summary
def allocations(base,m,checks,results,names):
    if m.get('matched_setup_controls'):
        require(len(checks)==2*len(results)*len(names),'paired allocation count')
        require(set(m['setup_results'])==set(results),'paired lanes')
        for lane,values in results.items():
            require(set(values)==set(names)==set(m['setup_results'][lane]),'paired workloads')
            for name,total in values.items():
                for count,expected in [(0,m['setup_results'][lane][name]),(1000,total)]:
                    matches=[c for c in checks if (c['lane'],c['workload'],c['actions'])==(lane,name,count)]
                    require(len(matches)==1,'unique paired probe');c=matches[0]
                    require(c['exit_code']==0 and not c['timed_out'],'paired exit')
                    require(c['extra_environment']==dict(STATECHRONICLE_BENCH_MULTIPLIER='1',STATECHRONICLE_ALLOCATION_PROBE='0' if count==0 else '1'),'paired probe environment')
                    require(c['command'][:3]==['taskset','-c',str(m['cpu_by_lane'][lane])] and c['command'][-2:]==[m['lanes'][lane]['binary_path'],name.ljust(128,'_')],'paired command')
                    data=(base/c['artifact']).read_bytes();require(sha(data)==c['sha256'],'paired log hash');raw=gzip.decompress(data)
                    require(re.search(rb'ERROR SUMMARY: 0 errors',raw),'paired memory errors')
                    match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);require(match is not None,'paired heap statistics')
                    actual=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));require(all(expected[k]==v for k,v in actual.items()),'paired totals')
                setup=m['setup_results'][lane][name]
                require(total['control_subtracted_per_action']=={k:(total[k]-setup[k])/1000 for k in ['allocations','bytes']},'workload-matched subtraction')
                require(all(v>=0 for v in total['control_subtracted_per_action'].values()),'negative action allocation')
            for name in ['native_validate_reduce','portable_validate_reduce_hash']:
                require(all(values[name]['control_subtracted_per_action'][k]==0 for k in ['allocations','bytes']),'paired allocation-free '+name)
        return len(checks)
    require(len(checks)==len(results)*len(names),'allocation count')
    for lane,values in results.items():
        require(set(values)==set(names),'allocation workloads')
        for name,total in values.items():
            matches=[c for c in checks if (c.get('lane')==lane and c.get('workload')==name) or ('lane' not in c and Path(c['artifact']).name in [lane+'-'+name+'.log.gz','allocation-'+lane+'-'+name+'.log.gz'])]
            require(len(matches)==1,'unique allocation log '+lane+' '+name);c=matches[0]
            require(c['exit_code']==0,'Valgrind exit');data=(base/c['artifact']).read_bytes();require(sha(data)==c['sha256'],'allocation hash');raw=gzip.decompress(data)
            require(re.search(rb'ERROR SUMMARY: 0 errors',raw),'Valgrind errors')
            match=re.search(rb'total heap usage:\s*([\d,]+) allocs,\s*([\d,]+) frees,\s*([\d,]+) bytes allocated',raw);require(match is not None,'heap statistics')
            actual=dict(zip(['allocations','frees','bytes'],[int(v.replace(b',',b'')) for v in match.groups()]));require(all(total[k]==v for k,v in actual.items()),'heap totals')
            control=values['control'];require(total['control_subtracted_per_action']=={k:(total[k]-control[k])/1000 for k in ['allocations','bytes']},'control subtraction')
        for name in ['native','portable','native_validate_reduce','portable_validate_reduce_hash']:
            if name in values:require(all(values[name][k]==values['control'][k] for k in ['allocations','bytes']),'allocation-free '+name)
    return len(checks)
after=load(out/'after-fuzz.json');complete=load(out/'complete-performance.json');repeat=load(out/'repeat-build-costs.json')
for m in [after,complete,repeat]:require(m['status']=='passed','prerequisite status')
matrix_path=Path(after['matrix_path']);matrix,logs=checked_manifest(root/matrix_path);matrix['_path']=str(matrix_path)
source_lanes(root/matrix_path,matrix,logs,matrix);observations(root/matrix_path,matrix,logs,26,True)
require(matrix['lanes']['dependency-control']['registry_features']==matrix['lanes']['candidate']['registry_features']==matrix['lanes']['completion-control']['registry_features'],'matched registry features')
paths_verified=[str(matrix_path)];allocation_checks=0
# Per-request path is emitted by its retained orchestration log.
raw=gzip.decompress((out/'per-request.log.gz').read_bytes()).decode()
request=Path(next(s for s in reversed(raw.splitlines()) if s.startswith('docs/0.3/evidence/performance/per-request-')))
for base,count in [(root/request,5),(root/complete['latency_path'],22)]:
    m,logs=checked_manifest(base);require(m['source_campaign_sha256']==sha((root/matrix_path/'manifest.json').read_bytes()),'matched source campaign')
    source_lanes(base,m,logs,matrix);observations(base,m,logs,count);paths_verified.append(str(base.relative_to(root)))
base=root/complete['latency_path'];a=load(base/'allocations.json');require(a['status']=='passed','all-path allocations');require(a['source_manifest_sha256']==sha((base/'manifest.json').read_bytes()),'allocation source binding');require(sha((base/'allocations.json').read_bytes())==complete['allocation_manifest_sha256'],'allocation orchestration binding')
require(a.get('matched_setup_controls') is True,'all-path workload-matched setup required')
require(a['source_campaign_sha256']==sha((root/a['source_campaign']/'manifest.json').read_bytes()),'paired source campaign')
for check in a['build_checks']:
    require(check['exit_code']==0,'paired build exit');data=(base/check['artifact']).read_bytes();require(sha(data)==check['sha256'],'paired build log hash')
    records=[json.loads(line) for line in gzip.decompress(data).splitlines() if line.startswith(b'{')];artifacts=[r for r in records if r.get('reason')=='compiler-artifact']
    require(artifacts and all(not r['fresh'] for r in artifacts),'paired fresh compilation')
    for crate in crates:require(any(r['target']['name']==crate.replace('-','_') for r in artifacts),'paired library compilation '+crate)
for lane,item in a['lanes'].items():
    require(item['source_sha256']==matrix['lanes'][lane]['source_sha256'],'paired source map')
    require(all(sha((Path(item['source_path'])/p).read_bytes())==h for p,h in item['source_sha256'].items()),'paired retained source')
    consumer=Path(item['consumer_path']);require({str(p.relative_to(consumer)):sha(p.read_bytes()) for p in consumer.rglob('*') if p.is_file()}==item['consumer_sha256'],'paired consumer map')
    code=(consumer/'crates/consumer/examples/refactor_bench.rs').read_bytes();require(sha(code)==item['benchmark_sha256'],'paired benchmark')
    old=(base/(lane+'-benchmark.rs')).read_text()
    before='''    if std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok().as_deref()==Some("1") {
        for _ in 0..1000 { action(); }
        std::process::exit(0);
    }'''
    after='''    let allocation_probe = std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok();
    if let Some(mode @ ("0" | "1")) = allocation_probe.as_deref() {
        let actions = if mode == "0" { 0 } else { 1000 };
        for _ in 0..actions { action(); }
        std::process::exit(0);
    }'''
    require(old.count(before)==1 and old.replace(before,after).encode()==code,'only zero-action control added')
    require(sha(Path(item['binary_path']).read_bytes())==item['binary_sha256'],'paired binary hash')
    require(sha((consumer/'crates/consumer/Cargo.lock').read_bytes())==item['lock_sha256']==matrix['lanes'][lane]['lock_sha256'],'paired lock')
allocation_checks+=allocations(base,a,a['checks'],a['results'],list(next(iter(a['results'].values()))))
# The smaller direct probes provide a second native/portable hard check.
raw=gzip.decompress((out/'allocations.log.gz').read_bytes()).decode();base=root/next(s for s in reversed(raw.splitlines()) if s.startswith('docs/0.3/evidence/performance/allocations-'))
m,logs=checked_manifest(base);allocation_checks+=allocations(base,m,m['checks'],m['results'],['control','native','portable','hosted','proof']);paths_verified.append(str(base.relative_to(root)))
base=out/'canonical-performance';m,logs=checked_manifest(base)
for lane,item in m['lanes'].items():
    data,files=archive(item['revision'],item['archive_paths']);require(sha(data)==item['archive_sha256'],'canonical archive');require({p:sha(b) for p,b in files.items()}==item['source_sha256'],'canonical source')
    require(sha((base/('bench-'+lane)).read_bytes())==item['binary_sha256'],'canonical binary')
    for path,h in m['fixture_sha256'].items():require(sha(subprocess.check_output(['git','show',m['candidate_commit']+':'+path]))==h,'canonical fixture')
summary=load(base/'summary.json');require(len(summary)==13,'canonical workloads')
for record in summary:
    name=record['workload']
    for lane,values in record['observations'].items():
        require(len(values)==7,'canonical cycles')
        for cycle,value in enumerate(values):
            sample_math(value);require(len(value['samples_ns'])==1000,'canonical sample count')
            require([v for v in json.loads(logs[str(cycle)+'-'+lane])['workloads'] if v['workload']==name]==[value],'canonical raw')
    for metric,deltas in record['paired_percent_deltas'].items():
        require(deltas==[(b[metric]/a[metric]-1)*100 for a,b in zip(record['observations']['control'],record['observations']['candidate'])],'canonical deltas');require(statistics.median(deltas)==record['median_paired_percent_delta'][metric],'canonical ratio median')
checks=[c for c in m['checks'] if c['label'].startswith('allocation-')];allocation_checks+=allocations(base,m,checks,m['allocations'],list(m['allocations']['control']));paths_verified.append(str(base.relative_to(root)))
result=dict(status='passed',verified_campaigns=paths_verified,allocation_probes=allocation_checks,limitations=['Numerical/source/log verification is separate from performance acceptance. Review thresholds and functionality/host effects in performance-review.md.','Native benchmark binaries are local verification inputs; hashes and reproducible source/logs are committed, executable build products are excluded.'])
(out/'performance-verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
