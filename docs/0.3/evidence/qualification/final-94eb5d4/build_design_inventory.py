"""Preserve additional literal H1-H8/work-map/discipline clauses for final audit."""
import hashlib,json,re
from pathlib import Path
root=Path.cwd();out=Path(__file__).resolve().parent;source=root/'docs/0.3/ROADMAP.md';text=source.read_text();items=[]
def add(identifier,requirement,review,evidence):items.append(dict(id=identifier,requirement=requirement,status='pending_final_audit',review=review,evidence=evidence))
for n in range(1,9):
 match=re.search(r'^### 3\.'+str(n)+r' H'+str(n)+r' .*?(?=^### |^## |\Z)',text,re.M|re.S);assert match
 review='api-review.md' if n in [4,5,7] else 'maintenance-review.md' if n in [3,6] else 'finding-disposition.md'
 evidence=['requirement-review.md','../local-release-94eb5d4/verification.json','semantic-verification.json','control-verification.json','performance-verification.json','requirement-inventory.json']
 for index,line in enumerate(re.findall(r'^- (.+)$',match[0],re.M),1):add(f'H{n}-Work-{index}',line,review,evidence)
 for label in ['Test/accept','Additional audit']:
  value=re.search(r'^\*\*'+re.escape(label)+r':\*\* (.+)$',match[0],re.M)
  if value:add(f'H{n}-'+label.replace('/','-').replace(' ','-'),value[1],review,evidence)
match=re.search(r'^## 4\. .*?(?=^## |\Z)',text,re.M|re.S);assert match
for index,row in enumerate([line for line in match[0].splitlines() if line.startswith('|') and not line.startswith(('| Target','|---'))],1):add('WorkMap-'+str(index),row.strip('|').strip(),'requirement-review.md',['maintenance-review.md','api-review.md','finding-disposition.md','../local-release-94eb5d4/verification.json','semantic-verification.json'])
match=re.search(r'^### 2\.1 .*?(?=^### |^## |\Z)',text,re.M|re.S);assert match
for index,line in re.findall(r'^(\d+)\. (.+)$',match[0],re.M):add('AuthorityQuestion-'+index,line,'api-review.md',['finding-disposition.md','docs/0.3/consumer-guide.md'])
for identifier,label in [('Footprint-Target','Footprint target'),('Conservative-Crates','Keep crate splitting conservative.'),('Change-Discipline','Change discipline'),('Stop-Investigate','Stop-and-investigate events')]:
 if label.endswith('.'):
  value=re.search(r'^\*\*'+re.escape(label)+r'\*\* (.+)$',text,re.M)
 else:value=re.search(r'^\*\*'+re.escape(label)+r':\*\* (.+)$',text,re.M)
 assert value,label
 add(identifier,value[1],'requirement-review.md',['maintenance-review.md','finding-disposition.md','performance-review.md','source-growth-accounting.json','metrics-verification.json'])
result=dict(roadmap_path=str(source.relative_to(root)),roadmap_sha256=hashlib.sha256(source.read_bytes()).hexdigest(),design_requirement_count=len(items),requirements=items,limitations=['Additional literal clauses complement the 138 numbered tasks and 84 supplemental clauses; overlap is retained, not counted as independent proof.','Conceptual example identifiers, suggested PR slices and tentative file decomposition are not mandatory APIs or authorization to publish/merge.','Unchanged protected-core files satisfy preservation tasks through original vectors/source equivalence; the work map does not require edits to every named path.','Human 5-10 minute onboarding is a usability target, not a measured human trial; the complete examples are independently built and reviewed.'])
(out/'design-requirements.json').write_text(json.dumps(result,indent=2)+'\n');print('Inventoried',len(items),'additional design/work-map/discipline clauses.')
