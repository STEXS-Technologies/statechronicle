"""Link overlapping supplemental/design clauses to specific reviewed numbered bases."""
import json
from pathlib import Path
out=Path(__file__).resolve().parent
numbered=json.loads((out/'requirement-inventory.json').read_text());index={r['id']:r for r in numbered['requirements']}
groups={
'Success':['DoD-1','F4-3','F3-2','7.4-1','F5-1','DoD-2','DoD-5'],
'NonGoal':['F6-7','F4-6','F6-9','F4-4','F3-4','F5-2'],
'Metric':['7.4-3','F6-1','7.4-6','F8-5','F3-1'],
'APIPrinciple':['F4-1','F4-3','F3-3','F1-2','F2-6','F4-1','F3-2','F3-5'],
'Performance':['F3-2','F3-2','F3-5','F8-5','F4-3','F1-2','F5-6','F2-3','F8-5'],
'Risk':['F1-4','F1-2','F1-5','7.1-1','7.2-9','F6-2','F3-2','7.3-7','F6-7','F6-8','F8-5','F5-2'],
'Distinction':['F2-3','F1-4','F4-5','7.2-2','F1-2','F1-1','F5-2','Scenario-D-5'],
'Maintainer':['F6-1','F6-2','F1-6','F0-2','F6-3','F8-2','F8-4','F8-6','7.2-4','F7-4','F6-8']}
links={f'{group}-{i}':[v] for group,values in groups.items() for i,v in enumerate(values,1)}
links.update({'Invariant-Transition':['F2-2','F3-2'],'Invariant-Identity':['F2-5'],'Invariant-Authority':['7.2-1','7.2-2'],'Invariant-Ledger':['7.2-4','7.2-5','7.2-6'],'Invariant-History/proofs':['7.1-2','F1-2'],'Invariant-Recovery':['F5-6','7.2-6'],'Invariant-Portability':['F3-5'],'Performance-Policy':['F8-5','F8-6','F3-5'],'Evidence-Policy':['F8-1','F8-3']})
for phase in range(9):links[f'F{phase}-Exit']=[f'F{phase}-{i}' for i in range(1,1+sum(r['id'].startswith(f'F{phase}-') for r in numbered['requirements']))]
# Publication is explicitly separate even when linked as part of the F8 exit.
files=['supplemental-requirements.json','design-requirements.json']
hwork={1:['F1-1','F1-2','F6-3','F4-7','F1-2'],2:['F4-1','F1-4','F1-5','F4-5','F4-3','F5-3'],3:['F2-1','F3-2','F2-2','F6-6','F3-5','F3-2'],4:['F3-3','F3-3','F3-4','F7-5','F3-4'],5:['F4-2','F4-6','F4-4','F4-5','F6-5','F7-2'],6:['F2-1','F2-7','F6-6','F2-3','F2-3','F6-3','F6-5'],7:['F4-1','F1-1','F4-4','F4-3','F5-6','F6-3'],8:['F5-1','F5-3','F5-2','F5-6','F5-3']}
for n,values in hwork.items():
 for i,v in enumerate(values,1):links[f'H{n}-Work-{i}']=[v]
links.update({f'H{i}-Test-accept':[v] for i,v in enumerate(['F1-6','F1-6','F3-2','F3-1','F4-6','F2-2','F1-2','F5-8'],1)})
links['H1-Additional-audit']=['F1-3']
for i,v in enumerate(['F4-1','F7-1','F3-2','F3-2','F3-3','F2-3','F6-3','F6-3','F2-7','F2-3','F4-3','F5-1','F4-5','F4-4','F1-1','F1-3','F0-3','F5-6','F6-9','F3-5','F8-3','F7-8'],1):links['WorkMap-'+str(i)]=[v]
for i,v in enumerate(['F2-1','7.2-1','F4-2','F1-1'],1):links['AuthorityQuestion-'+str(i)]=[v]
links.update({'Footprint-Target':['F6-8','7.4-4'],'Conservative-Crates':['F6-7'],'Change-Discipline':['F8-1','F6-2'],'Stop-Investigate':['DoD-3','F8-6']})
for file in files:
 p=out/file;j=json.loads(p.read_text())
 for row in j['requirements']:
  refs=links[row['id']];assert refs and all(v in index for v in refs)
  row['reviewed_with_numbered_clauses']=refs
  row['acceptance_basis']=' '.join(index[v]['acceptance_basis'] for v in refs)
  if row['id'] in ['H4-Test-accept','Metric-5']:row['evidence']+=['consumer-setup-metrics.json','metrics-verification.json']
  row['basis_scope']='Literal clause reviewed with the cited numbered bases and named review. Shared evidence is not an independent second test. Status remains pending until full terminal audit.'
 p.write_text(json.dumps(j,indent=2)+'\n')
 print(file,len(j['requirements']),'clause-specific review links recorded')
