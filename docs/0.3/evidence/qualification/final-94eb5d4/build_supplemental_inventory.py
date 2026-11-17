"""Capture normative roadmap clauses beyond the 138 phase/checklist tasks."""
import hashlib,json,re
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;source=root/'docs/0.3/ROADMAP.md';text=source.read_text();items=[]
def add(identifier,requirement,review,evidence):items.append(dict(id=identifier,requirement=requirement,status='pending_final_audit',review=review,evidence=evidence))
def section(heading):
    match=re.search(r'^'+re.escape(heading)+r'\n(.*?)(?=^#{1,3} |\Z)',text,re.M|re.S);assert match,heading;return match[1]
for number,line in enumerate(re.findall(r'^\d+\. (.+)$',section('## 0. Executive decision'),re.M),1):add('Success-'+str(number),line,'requirement-review.md',['maintenance-review.md','api-review.md','performance-verification.json'])
for number,line in enumerate(re.findall(r'^- (.+)$',section('### Non-goals'),re.M),1):add('NonGoal-'+str(number),line,'requirement-review.md',['layer-review.json','api-review.md','maintenance-review.md'])
for name,line in re.findall(r'^\*\*([^*]+):\*\* (.+?)(?:\\)?$',section('### 1.1 Invariants the refactor must not lose'),re.M):add('Invariant-'+name,line.rstrip('\\'),'requirement-review.md',['../local-release-94eb5d4/verification.json','control-verification.json','semantic-verification.json','performance-verification.json'])
for number,line in enumerate(re.findall(r'^- (.+)$',section('### 1.2 Why smaller code is a real deliverable'),re.M),1):add('Metric-'+str(number),line,'maintenance-review.md',['source-metrics.json','baseline-source-metrics.json','build-cost-verification.json','performance-verification.json'])
for number,line in enumerate(re.findall(r'^- (.+)$',section('### 2.2 Public API design principles'),re.M),1):add('APIPrinciple-'+str(number),line,'api-review.md',['api-comparison.json','api-structural-diff.json','../local-release-94eb5d4/verification.json'])
for phase in range(9):
    match=re.search(r'^### F'+str(phase)+r' — .*?(?=^### F\d|^## \d|\Z)',text,re.M|re.S);assert match
    line=re.search(r'^\*\*Exit:\*\* (.+)$',match[0],re.M);assert line
    add('F'+str(phase)+'-Exit',line[1],'requirement-review.md',['requirement-inventory.json','performance-verification.json','build-cost-verification.json'])
for row in section('### 7.5 Performance gate').splitlines():
    if row.startswith('|') and not row.startswith(('| Path','|---')):
        fields=[f.strip() for f in row.strip('|').split('|')];add('Performance-'+str(sum(v['id'].startswith('Performance-') for v in items)+1),' | '.join(fields),'performance-review.md',['performance-verification.json','after-fuzz.json','complete-performance.json'])
add('Performance-Policy','Matched compiler/dependency controls, warmups, alternated order, pinned CPU/config and hashes; review repeatable >5% median or >10% p95 regressions against noise/functionality; portable allocations are a hard failure.','performance-review.md',['performance-verification.json','build-cost-verification.json'])
add('Evidence-Policy','Retain exact revisions/toolchains/commands/environment, input/artifact hashes, actual exits, scope and limitations; preserve failed/interrupted attempts without reclassifying them.','requirement-review.md',['baseline-verification.json','control-verification.json','performance-verification.json','candidate-equivalence.json'])
for row in section('## 8. Risks, non-equivalences, and stop conditions').splitlines():
    if row.startswith('|') and not row.startswith(('| Risk','|---')):
        add('Risk-'+str(sum(v['id'].startswith('Risk-') for v in items)+1),row.strip('|').strip(),'requirement-review.md',['control-verification.json','semantic-verification.json','performance-verification.json','../local-release-94eb5d4/verification.json'])
for number,line in enumerate(re.findall(r'^- (.+)$',section('### Distinctions that must remain visible'),re.M),1):add('Distinction-'+str(number),line,'api-review.md',['docs/0.3/consumer-guide.md','docs/0.3/adapter-contract.md','docs/0.3/durable-outcomes.md','docs/0.3/proof-claims.md'])
for number,line in enumerate(re.findall(r'^- \[ \] (.+)$',section('## 10. Maintainer checklist for each work item'),re.M),1):add('Maintainer-'+str(number),line,'requirement-review.md',['docs/0.3/maintenance-inventory.md','docs/0.3/findings.md','docs/0.3/evidence/maintenance/','docs/0.3/evidence/f2/'])
result=dict(roadmap_path=str(source.relative_to(root)),roadmap_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),supplemental_requirement_count=len(items),requirements=items,limitations=['These clauses supplement, not replace, the 138 literal numbered phase/gate/scenario/definition-of-done tasks.','Design sketches and provisional names are reviewed against documented equivalent final APIs, not treated as exact required identifiers.','Status remains pending until terminal evidence and the manual aggregate review close it.'])
(out/'supplemental-requirements.json').write_text(json.dumps(result,indent=2)+'\n');print('Inventoried',len(items),'additional explicit success/design/invariant/exit/performance/risk/evidence/maintenance clauses')
