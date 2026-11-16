import hashlib,json,pathlib,subprocess
from tree_sitter import Language,Parser
import tree_sitter_rust
root=pathlib.Path.cwd();out=root/'docs/0.3/evidence/maintenance/string-traits';candidate=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip();control='dea8a076d3e4e0e6c535a8dd1a2dbc7451688b49'
parser=Parser(Language(tree_sitter_rust.language()))
def sha(b):return hashlib.sha256(b).hexdigest()
def text(node,data):return data[node.start_byte:node.end_byte].decode()
def items(revision,module,names):
 data=subprocess.check_output(['git','show',f'{revision}:crates/statechronicle-domain/src/{module}.rs']);tree=parser.parse(data)
 if tree.root_node.has_error:raise RuntimeError('parse failure')
 result={}
 for node in tree.root_node.children:
  if node.type=='struct_item':
   name=text(node.child_by_field_name('name'),data)
   if name in names:
    result['struct:'+name]=sha(data[node.start_byte:node.end_byte])
    attributes=[];previous=node.prev_named_sibling
    while previous is not None and previous.type in {'attribute_item','line_comment','block_comment'}:
     if previous.type=='attribute_item':attributes.append(text(previous,data))
     previous=previous.prev_named_sibling
    result['attributes:'+name]=sha('\n'.join(reversed(attributes)).encode())
  if node.type=='impl_item' and node.child_by_field_name('trait') is None:
   typ=node.child_by_field_name('type')
   if typ and text(typ,data) in names:
    name=text(typ,data)
    for child in node.child_by_field_name('body').children:
     if child.type=='function_item':result[name+'::'+text(child.child_by_field_name('name'),data)]=sha(data[child.start_byte:child.end_byte])
  if node.type=='function_item':
   name=text(node.child_by_field_name('name'),data)
   if name.startswith('validate_'):result['validator:'+name]=sha(data[node.start_byte:node.end_byte])
 return result
mapping={'ids':['StateId','IntentId','EventId','CommitId','SnapshotId'],'commit':['ProfileId'],'intent':['Operation','KeyId'],'status':['Status']};checks={}
for module,names in mapping.items():
 before=items(control,module,names);after=items(candidate,module,names)
 if not before or before!=after:raise RuntimeError('type/constructor/validator drift '+module)
 checks[module]=after
m={'status':'passed','control':control,'candidate':candidate,'method':'tree-sitter Rust top-level structs, adjacent attributes, inherent methods and validators; exact byte hashes excluding adjacent comments','checks':checks,'limitations':['Trait shells are characterized through the external compiler/runtime corpus, not macro-expanded by this AST audit.','Status FromStr and borrowed TryFrom previously delegate through unchanged try_from_str; that helper is exactly Self::new(String::from(value)).']}
(out/'owner-audit.py').write_bytes(pathlib.Path(__file__).read_bytes());(out/'owner-audit.json').write_text(json.dumps(m,indent=2)+'\n');print('verified',sum(map(len,checks.values())),'owner definitions')
