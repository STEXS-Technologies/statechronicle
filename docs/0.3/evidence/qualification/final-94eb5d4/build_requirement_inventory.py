"""Freeze the roadmap's numbered phase tasks, release gates and acceptance clauses."""
import hashlib,json,re
from pathlib import Path
root=Path.cwd();source=root/'docs/0.3/ROADMAP.md';text=source.read_text();items=[]
phase_evidence={
 'F0':['docs/0.3/baseline.md','docs/0.3/evidence/api/compiler-c418e98/manifest.json','ci/compat-0.2/provenance.json','ci/wire-0.2/provenance.json','docs/0.3/evidence/compat/20261116T180227Z-1914823/manifest.json','baseline-source-metrics.json','after-fuzz.json'],
 'F1':['../local-release-94eb5d4/release.log.gz','../../fuzz/claim-final-0799afe/manifest.json','../../fuzz/durable-final-0799afe/manifest.json','docs/0.3/durable-outcomes.md','docs/0.3/proof-claims.md'],
 'F2':['../local-release-94eb5d4/release.log.gz','maintenance-review.md','docs/0.3/preparation-pipeline.md','docs/0.3/batch-preparation.md'],
 'F3':['../local-release-94eb5d4/verification.json','../local-release-94eb5d4/nested/','docs/0.3/typed-command-authoring.md','../../compat/20261116T180227Z-1914823/manifest.json'],
 'F4':['api-review.md','canonical-performance/manifest.json','../local-release-94eb5d4/nested/','docs/0.3/consumer-guide.md','docs/0.3/migration.md'],
 'F5':['../../conformance/concurrent-mixed-0799afe/all-tests.log.gz','../../conformance/concurrent-mixed-0799afe/deadlock-control/manifest.json','process-lock-control/lock-negative.json','../local-release-94eb5d4/nested/'],
 'F6':['source-metrics.json','baseline-source-metrics.json','maintenance-review.md','layer-review.json','after-fuzz.json','docs/0.3/evidence/maintenance/'],
 'F7':['CHANGELOG.md','docs/0.3/release-compatibility.md','docs/0.3/consumer-guide.md','docs/0.3/migration.md','docs/OPERATIONS/RECOVERY.md','../local-release-94eb5d4/nested/'],
 'F8':['../local-release-94eb5d4/verification.json','../../packages/20261116T180422Z-1975357/manifest.json','../../coverage/hosted-94eb5d4/manifest.json','../../fuzz/extended-94eb5d4/manifest.json','properties/manifest.json','after-fuzz.json','api-review.md','candidate-equivalence.json']}

def add(identifier,requirement,evidence):
 items.append(dict(id=identifier,requirement=requirement,status='pending_final_audit',evidence=evidence))
for phase in range(9):
 name='F'+str(phase)
 match=re.search(r'^### '+name+r' — .*?(?=^### F\d|^## \d|\Z)',text,re.M|re.S)
 assert match,name
 for number,requirement in re.findall(r'^(\d+)\. (.+)$',match[0],re.M):
  add(name+'-'+number,requirement,phase_evidence[name])
for gate in ['7.1','7.2','7.3','7.4']:
 match=re.search(r'^### '+re.escape(gate)+r' .*?(?=^### |^## |\Z)',text,re.M|re.S)
 for number,requirement in enumerate(re.findall(r'^- \[ \] (.+)$',match[0],re.M),1):
  evidence=phase_evidence[{'7.1':'F0','7.2':'F1','7.3':'F4','7.4':'F6'}[gate]]
  add(gate+'-'+str(number),requirement,evidence)
for scenario in 'ABCDE':
 match=re.search(r'^### Scenario '+scenario+r' — .*?(?=^### |^## |\Z)',text,re.M|re.S)
 for number,requirement in re.findall(r'^(\d+)\. (.+)$',match[0],re.M):
  evidence=phase_evidence['F5'] if scenario in 'CD' else phase_evidence['F3' if scenario in 'AE' else 'F1']
  add('Scenario-'+scenario+'-'+number,requirement,evidence)
match=re.search(r'^## 11\. .*?(?=^## |\Z)',text,re.M|re.S)
for number,requirement in re.findall(r'^(\d+)\. (.+)$',match[0],re.M):add('DoD-'+number,requirement,phase_evidence['F8'])
for item in items:
 if item['id']=='F8-9':
  item.update(status='separate_release_action',reason='User authorized qualification, commit and push. Version bump, tag and registry publishing are separate actions under the roadmap.')
out=Path(__file__).resolve().parent/'requirement-inventory.json'
out.write_text(json.dumps(dict(roadmap_path=str(source.relative_to(root)),roadmap_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),requirements=items,requirement_count=len(items),limitations=['Inventory preserves literal numbered tasks and checklist clauses; provisional names and optional design choices are assessed against final documented equivalents.','No completion is implied by a path or this inventory: raw evidence and every active gate require independent final review.']),indent=2)+'\n')
print('Inventoried',len(items),'explicit roadmap phase tasks, release checks, scenarios and definition-of-done clauses.')
